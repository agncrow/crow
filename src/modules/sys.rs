//! `use sys`: the standard library's own layer — bytes, and a connection
//! to a host read and written as bytes — rows of the builtin table
//! (crate::table), their runtime functions in `crow_runtime::sys`, which
//! both modes call. A program may not import it: only a module of the
//! standard library (the spec, "The standard library's layer").
use crate::modules::CrowModule;

pub struct SysModule;

pub(crate) const SYS_FUNCTIONS: &[&str] = &["open", "send", "recv", "close", "bytes", "text", "slice", "find", "concat", "byte", "gather", "file", "store", "random", "hex", "hash", "hmac", "reply", "answer", "csv_rows", "frame_of_csv", "frame_of_json", "frame_of_parquet", "frame_csv", "frame_json", "frame_parquet", "tensor_of_frame", "exchange", "fetch", "put", "format_of", "format_sniffed", "remote", "with_query", "redirected"];

impl CrowModule for SysModule {
    fn name(&self) -> &str { "sys" }

    fn functions(&self) -> &[&str] { SYS_FUNCTIONS }
}
