/// Distributed computing runtime for Crow.
/// TCP-based communication with ring allreduce for gradient synchronization.
/// Process discovery via environment variables:
///   CROW_RANK, CROW_WORLD_SIZE, CROW_MASTER_ADDR, CROW_MASTER_PORT

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Mutex, OnceLock};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use crate::tensor::CrowTensor;

/// Global distributed state
static DIST_INITIALIZED: AtomicBool = AtomicBool::new(false);
static DIST_RANK: AtomicUsize = AtomicUsize::new(0);
static DIST_WORLD_SIZE: AtomicUsize = AtomicUsize::new(1);
static DIST_PEERS: OnceLock<Mutex<DistPeers>> = OnceLock::new();

/// Peer connections for ring topology
struct DistPeers {
    /// Connection to the next rank in the ring (rank+1 % world_size)
    send_stream: Option<TcpStream>,
    /// Connection from the previous rank in the ring (rank-1+world_size % world_size)
    recv_stream: Option<TcpStream>,
}

/// Check if distributed mode is enabled.
pub fn is_distributed() -> bool {
    DIST_INITIALIZED.load(Ordering::SeqCst)
}

/// A cluster that cannot form, or a peer that stops answering, is the
/// machine failing, not the program: the process stops with the reason, the
/// same in both modes, and never panics (docs/bug-classes.md §3).
fn fail(why: String) -> ! {
    crate::stops::halt(0, &why)
}

const NOT_STARTED: &str = "the cluster was not started: nodes[n] starts it";

/// Eight bytes of a message from another rank at `pos`, or the stop a
/// message cut short is.
fn word(bytes: &[u8], pos: usize) -> [u8; 8] {
    match bytes.get(pos..pos + 8).and_then(|b| <[u8; 8]>::try_from(b).ok()) {
        Some(w) => w,
        None => fail(format!("rank {}: a message from another rank was cut short", rank())),
    }
}

/// Get the rank of this process.
pub fn rank() -> usize {
    DIST_RANK.load(Ordering::SeqCst)
}

/// Get the world size (total number of processes).
pub fn world_size() -> usize {
    DIST_WORLD_SIZE.load(Ordering::SeqCst)
}

/// Initialize the distributed runtime.
/// Reads CROW_RANK, CROW_WORLD_SIZE, CROW_MASTER_ADDR, CROW_MASTER_PORT from environment.
/// Sets up TCP ring topology for allreduce.
pub fn init(expected_world_size: usize) {
    if DIST_INITIALIZED.load(Ordering::SeqCst) {
        return; // already initialized
    }

    let rank: usize = std::env::var("CROW_RANK")
        .unwrap_or_else(|_| "0".to_string())
        .parse()
        .unwrap_or_else(|_| fail("CROW_RANK must be a non-negative integer".to_string()));

    let ws: usize = std::env::var("CROW_WORLD_SIZE")
        .unwrap_or_else(|_| expected_world_size.to_string())
        .parse()
        .unwrap_or_else(|_| fail("CROW_WORLD_SIZE must be a positive integer".to_string()));

    let master_addr = std::env::var("CROW_MASTER_ADDR")
        .unwrap_or_else(|_| "127.0.0.1".to_string());

    let base_port: u16 = std::env::var("CROW_MASTER_PORT")
        .unwrap_or_else(|_| "29500".to_string())
        .parse()
        .unwrap_or_else(|_| fail("CROW_MASTER_PORT must be a valid port number".to_string()));

    DIST_RANK.store(rank, Ordering::SeqCst);
    DIST_WORLD_SIZE.store(ws, Ordering::SeqCst);

    if ws <= 1 {
        // Single process — no networking needed
        DIST_INITIALIZED.store(true, Ordering::SeqCst);
        DIST_PEERS.get_or_init(|| Mutex::new(DistPeers {
            send_stream: None,
            recv_stream: None,
        }));
        return;
    }

    // Set up ring topology:
    // Each rank listens on base_port + rank
    // Each rank connects to (rank + 1) % world_size
    let my_port = base_port + rank as u16;
    let next_rank = (rank + 1) % ws;
    let next_port = base_port + next_rank as u16;

    // Start listener for incoming connection from previous rank
    let listener = TcpListener::bind(format!("{}:{}", master_addr, my_port))
        .unwrap_or_else(|e| fail(format!("rank {}: failed to bind port {}: {}", rank, my_port, e)));

    // Connect to next rank (with retries)
    let send_stream = connect_with_retry(&master_addr, next_port, rank, next_rank);

    // Accept connection from previous rank
    let (recv_stream, _addr) = listener.accept()
        .unwrap_or_else(|e| fail(format!("rank {}: failed to accept connection: {}", rank, e)));

    DIST_PEERS.get_or_init(|| Mutex::new(DistPeers {
        send_stream: Some(send_stream),
        recv_stream: Some(recv_stream),
    }));

    DIST_INITIALIZED.store(true, Ordering::SeqCst);

    // Barrier to ensure all ranks are ready
    barrier();
}

fn connect_with_retry(addr: &str, port: u16, my_rank: usize, target_rank: usize) -> TcpStream {
    let target = format!("{}:{}", addr, port);
    for attempt in 0..60 {
        match TcpStream::connect(&target) {
            Ok(stream) => return stream,
            Err(_) => {
                if attempt % 10 == 9 {
                    eprintln!("rank {}: waiting for rank {} at {}...", my_rank, target_rank, target);
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        }
    }
    fail(format!("rank {}: failed to connect to rank {} at {} after 6 seconds", my_rank, target_rank, target))
}

/// Send raw bytes over the ring to the next rank.
fn send_bytes(data: &[u8]) {
    let peers = DIST_PEERS.get().unwrap_or_else(|| fail(NOT_STARTED.to_string()));
    let mut peers = crate::lock(peers);
    if let Some(ref mut stream) = peers.send_stream {
        // Send length prefix (8 bytes, little-endian)
        let len = data.len() as u64;
        let sent = stream.write_all(&len.to_le_bytes()).and_then(|_| stream.write_all(data)).and_then(|_| stream.flush());
        if let Err(e) = sent {
            fail(format!("rank {}: sending to the next rank failed: {}", rank(), e));
        }
    }
}

/// Receive raw bytes from the previous rank in the ring.
fn recv_bytes() -> Vec<u8> {
    let peers = DIST_PEERS.get().unwrap_or_else(|| fail(NOT_STARTED.to_string()));
    let mut peers = crate::lock(peers);
    if let Some(ref mut stream) = peers.recv_stream {
        let mut len_buf = [0u8; 8];
        if let Err(e) = stream.read_exact(&mut len_buf) {
            fail(format!("rank {}: receiving from the previous rank failed: {}", rank(), e));
        }
        let len = u64::from_le_bytes(len_buf) as usize;
        let mut data = vec![0u8; len];
        if let Err(e) = stream.read_exact(&mut data) {
            fail(format!("rank {}: receiving from the previous rank failed: {}", rank(), e));
        }
        data
    } else {
        Vec::new()
    }
}

/// Serialize a tensor to bytes: shape_len(u64) + shape(u64 each) + data(f64 each)
fn tensor_to_bytes(t: &CrowTensor) -> Vec<u8> {
    let shape = t.shape().to_vec();
    let data = t.to_vec();
    let mut buf = Vec::new();
    // shape length
    buf.extend_from_slice(&(shape.len() as u64).to_le_bytes());
    // shape values
    for &s in &shape {
        buf.extend_from_slice(&(s as u64).to_le_bytes());
    }
    // data length
    buf.extend_from_slice(&(data.len() as u64).to_le_bytes());
    // data values
    for &v in &data {
        buf.extend_from_slice(&v.to_le_bytes());
    }
    buf
}

/// Deserialize a tensor from bytes.
fn tensor_from_bytes(bytes: &[u8]) -> CrowTensor {
    let mut pos = 0;
    // shape length
    let shape_len = u64::from_le_bytes(word(bytes, pos)) as usize;
    pos += 8;
    // shape values
    let mut shape = Vec::with_capacity(shape_len);
    for _ in 0..shape_len {
        shape.push(u64::from_le_bytes(word(bytes, pos)) as usize);
        pos += 8;
    }
    // data length
    let data_len = u64::from_le_bytes(word(bytes, pos)) as usize;
    pos += 8;
    // data values
    let mut data = Vec::with_capacity(data_len);
    for _ in 0..data_len {
        data.push(f64::from_le_bytes(word(bytes, pos)));
        pos += 8;
    }
    CrowTensor::from_vec(data, shape)
}

/// Ring allreduce: average a tensor across all ranks.
/// Uses a two-phase ring algorithm: scatter-reduce then allgather.
/// Returns the averaged tensor (sum / world_size).
pub fn allreduce_mean(tensor: &CrowTensor) -> CrowTensor {
    let ws = world_size();
    if ws <= 1 {
        return tensor.clone();
    }

    let data = tensor.to_vec();
    let n = data.len();

    // Chunk size for ring: split data into ws chunks
    let chunk_size = (n + ws - 1) / ws;
    let r = rank();

    // Phase 1: Scatter-reduce
    // Each process starts with its own data. In each step, send one chunk to next,
    // receive one chunk from prev, and accumulate.
    let mut accum = data.clone();

    for step in 0..(ws - 1) {
        // Send chunk index: (rank - step + ws) % ws
        let send_idx = (r + ws - step) % ws;
        let send_start = send_idx * chunk_size;
        let send_end = std::cmp::min(send_start + chunk_size, n);

        // Send the chunk
        let chunk: Vec<u8> = accum[send_start..send_end].iter()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        send_bytes(&chunk);

        // Receive chunk from previous rank
        let recv_data = recv_bytes();
        let recv_idx = (r + ws - step - 1) % ws;
        let recv_start = recv_idx * chunk_size;
        let _recv_end = std::cmp::min(recv_start + chunk_size, n);

        // Accumulate
        let recv_chunk: Vec<f64> = recv_data.chunks_exact(8)
            .map(|b| f64::from_le_bytes(b.try_into().unwrap_or_default()))
            .collect();
        for (i, &v) in recv_chunk.iter().enumerate() {
            if recv_start + i < n {
                accum[recv_start + i] += v;
            }
        }
    }

    // Phase 2: Allgather — distribute the fully reduced chunks
    for step in 0..(ws - 1) {
        let send_idx = (r + 1 + ws - step) % ws;
        let send_start = send_idx * chunk_size;
        let send_end = std::cmp::min(send_start + chunk_size, n);

        let chunk: Vec<u8> = accum[send_start..send_end].iter()

            .flat_map(|v| v.to_le_bytes())
            .collect();
        send_bytes(&chunk);

        let recv_data = recv_bytes();
        let recv_idx = (r + ws - step) % ws;
        let recv_start = recv_idx * chunk_size;
        let _recv_end = std::cmp::min(recv_start + chunk_size, n);

        let recv_chunk: Vec<f64> = recv_data.chunks_exact(8)
            .map(|b| f64::from_le_bytes(b.try_into().unwrap_or_default()))
            .collect();
        for (i, &v) in recv_chunk.iter().enumerate() {
            if recv_start + i < n {
                accum[recv_start + i] = v;
            }
        }
    }

    // Divide by world_size to get mean
    let ws_f = ws as f64;
    let result: Vec<f64> = accum.iter().map(|v| v / ws_f).collect();
    CrowTensor::from_vec(result, tensor.shape().to_vec())
}

/// Broadcast a tensor from the root rank (rank 0) to all other ranks.
/// Uses a simple ring pass: root sends, each rank forwards to next.
pub fn broadcast(tensor: &CrowTensor, root: usize) -> CrowTensor {
    let ws = world_size();
    if ws <= 1 {
        return tensor.clone();
    }

    let r = rank();
    let distance_from_root = (r + ws - root) % ws;

    if distance_from_root == 0 {
        // Root: send tensor to next rank, then wait for it to come back
        let bytes = tensor_to_bytes(tensor);
        send_bytes(&bytes);
        let _ = recv_bytes(); // drain the ring completion
        tensor.clone()
    } else {
        // Non-root: receive from prev, forward to next
        let data = recv_bytes();
        send_bytes(&data);
        tensor_from_bytes(&data)
    }
}

/// Barrier: synchronize all ranks.
/// Uses a simple ring-based barrier.
pub fn barrier() {
    let ws = world_size();
    if ws <= 1 {
        return;
    }
    // Send a single byte around the ring
    send_bytes(&[0u8; 1]);
    let _ = recv_bytes();
    // Second round to ensure all have passed
    send_bytes(&[0u8; 1]);
    let _ = recv_bytes();
}

/// Shard a tensor along the first axis across all ranks.
/// Each rank gets its portion: tensor[rank * chunk_size .. (rank+1) * chunk_size]
pub fn shard(tensor: &CrowTensor) -> CrowTensor {
    let ws = world_size();
    if ws <= 1 {
        return tensor.clone();
    }

    let shape = tensor.shape().to_vec();
    if shape.is_empty() {
        return tensor.clone();
    }

    let first_dim = shape[0];
    let chunk_size = (first_dim + ws - 1) / ws;
    let r = rank();
    let start = r * chunk_size;
    let end = std::cmp::min(start + chunk_size, first_dim);

    if start >= first_dim {
        // This rank gets nothing — return empty tensor
        let mut new_shape = shape.clone();
        new_shape[0] = 0;
        return CrowTensor::from_vec(vec![], new_shape);
    }

    // Extract rows [start..end]
    let data = tensor.to_vec();
    let row_size: usize = shape[1..].iter().product();
    let row_size = if row_size == 0 { 1 } else { row_size };
    let slice = data[start * row_size..end * row_size].to_vec();

    let mut new_shape = shape;
    new_shape[0] = end - start;
    CrowTensor::from_vec(slice, new_shape)
}

/// Finalize the distributed runtime.
pub fn finalize() {
    if !DIST_INITIALIZED.load(Ordering::SeqCst) {
        return;
    }
    // Barrier before cleanup
    barrier();
    // Drop connections
    if let Some(peers) = DIST_PEERS.get() {
        let mut peers = crate::lock(&peers);
        peers.send_stream = None;
        peers.recv_stream = None;
    }
}

/// Display helper for distributed info.
pub fn info() -> String {
    if is_distributed() {
        format!("rank {}/{}", rank(), world_size())
    } else {
        "not distributed".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // All tests in a single function to avoid global state races
    // (tests share DIST_RANK, DIST_WORLD_SIZE, etc. atomics)
    #[test]
    fn test_distributed_all() {
        // --- tensor serialization roundtrip ---
        let t = CrowTensor::from_vec(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0], vec![2, 3]);
        let bytes = tensor_to_bytes(&t);
        let t2 = tensor_from_bytes(&bytes);
        assert_eq!(t.to_vec(), t2.to_vec());
        assert_eq!(t.shape(), t2.shape());

        // --- allreduce single process ---
        DIST_WORLD_SIZE.store(1, Ordering::SeqCst);
        DIST_RANK.store(0, Ordering::SeqCst);
        let t = CrowTensor::from_vec(vec![1.0, 2.0, 3.0], vec![3]);
        let result = allreduce_mean(&t);
        assert_eq!(result.to_vec(), vec![1.0, 2.0, 3.0]);

        // --- shard single process ---
        DIST_WORLD_SIZE.store(1, Ordering::SeqCst);
        let t = CrowTensor::from_vec(vec![1.0, 2.0, 3.0, 4.0], vec![4]);
        let result = shard(&t);
        assert_eq!(result.to_vec(), vec![1.0, 2.0, 3.0, 4.0]);

        // --- shard two processes rank 0 ---
        DIST_WORLD_SIZE.store(2, Ordering::SeqCst);
        DIST_RANK.store(0, Ordering::SeqCst);
        let t = CrowTensor::from_vec(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0], vec![3, 2]);
        let result = shard(&t);
        assert_eq!(result.to_vec(), vec![1.0, 2.0, 3.0, 4.0]);
        assert_eq!(result.shape(), vec![2, 2]);

        // --- shard two processes rank 1 ---
        DIST_RANK.store(1, Ordering::SeqCst);
        let result = shard(&t);
        assert_eq!(result.to_vec(), vec![5.0, 6.0]);
        assert_eq!(result.shape(), vec![1, 2]);

        // --- info not distributed ---
        DIST_INITIALIZED.store(false, Ordering::SeqCst);
        DIST_RANK.store(0, Ordering::SeqCst);
        DIST_WORLD_SIZE.store(1, Ordering::SeqCst);
        assert_eq!(info(), "not distributed");

        // --- info distributed ---
        DIST_INITIALIZED.store(true, Ordering::SeqCst);
        DIST_RANK.store(2, Ordering::SeqCst);
        DIST_WORLD_SIZE.store(4, Ordering::SeqCst);
        assert_eq!(info(), "rank 2/4");

        // --- barrier single process ---
        DIST_WORLD_SIZE.store(1, Ordering::SeqCst);
        DIST_RANK.store(0, Ordering::SeqCst);
        DIST_INITIALIZED.store(true, Ordering::SeqCst);
        DIST_PEERS.get_or_init(|| Mutex::new(DistPeers {
            send_stream: None,
            recv_stream: None,
        }));
        barrier(); // should not hang

        // Reset
        DIST_INITIALIZED.store(false, Ordering::SeqCst);
        DIST_RANK.store(0, Ordering::SeqCst);
        DIST_WORLD_SIZE.store(1, Ordering::SeqCst);
    }
}

