//! The messages Postgres answers a query with, decoded straight into a
//! frame: the kernel a driver written in Crow hands the bytes it read
//! after sending the query (`sys.frame_of_postgres`) — the row
//! description (`T`), the rows (`D`), the command's completion (`C`) and
//! the server's ready-for-query (`Z`). A column is typed by the type the
//! description names: the integers an int column, floats and numerics a
//! float column, booleans a bool column, dates and timestamps a datetime
//! column in UTC milliseconds (a timestamp without a zone read as UTC, one with a zone keeping
//! its offset),
//! anything else text; a NULL is a missing cell. An error the server
//! answers (`E`) is the reason, in its words; bytes that end before the
//! ready-for-query are refused as ending early, which is how a driver
//! knows to read more; a cell sent in the binary format is refused.
use crate::frame::{Column, CrowFrame};

/// A column being filled, by the kind its type names.
enum Col {
    Int(Vec<i64>),
    Float(Vec<f64>),
    Str(Vec<String>),
    Bool(Vec<bool>),
    /// UTC milliseconds and the offset each was written in
    DateTime(Vec<i64>, Vec<i16>),
}

fn be32(b: &[u8]) -> i64 {
    i32::from_be_bytes([b[0], b[1], b[2], b[3]]) as i64
}

fn be16(b: &[u8]) -> usize {
    u16::from_be_bytes([b[0], b[1]]) as usize
}

/// The column a type's cells fill, by the type's OID.
fn col_of(oid: i64, n: usize) -> Col {
    match oid {
        // int2, int8, int4, oid
        21 | 20 | 23 | 26 => Col::Int(Vec::with_capacity(n)),
        // float4, float8, numeric
        700 | 701 | 1700 => Col::Float(Vec::with_capacity(n)),
        16 => Col::Bool(Vec::with_capacity(n)),
        // date, timestamp, timestamptz
        1082 | 1114 | 1184 => Col::DateTime(Vec::with_capacity(n), Vec::with_capacity(n)),
        _ => Col::Str(Vec::with_capacity(n)),
    }
}

/// A C string's text and what follows it.
fn cstring(b: &[u8]) -> Result<(&str, &[u8]), String> {
    let end = b.iter().position(|&c| c == 0).ok_or("a name without its end")?;
    let s = std::str::from_utf8(&b[..end]).map_err(|_| "a name that is not UTF-8".to_string())?;
    Ok((s, &b[end + 1..]))
}

/// The frame the bytes hold, or why they do not.
pub fn frame_of_postgres(b: &[u8]) -> Result<CrowFrame, String> {
    let mut names: Vec<String> = Vec::new();
    let mut oids: Vec<i64> = Vec::new();
    let mut cols: Vec<Col> = Vec::new();
    let mut valids: Vec<Vec<bool>> = Vec::new();
    let mut described = false;
    let mut i = 0;
    loop {
        if i + 5 > b.len() {
            return Err(format!("the answer ends early at byte {}", i));
        }
        let kind = b[i];
        let len = be32(&b[i + 1..]);
        if len < 4 {
            return Err(format!("a message of length {} at byte {}", len, i));
        }
        let end = i + 1 + len as usize;
        if end > b.len() {
            return Err(format!("the answer ends early at byte {}", b.len()));
        }
        let body = &b[i + 5..end];
        match kind {
            b'T' => {
                if body.len() < 2 {
                    return Err("a row description without its count".into());
                }
                let n = be16(body);
                let mut rest = &body[2..];
                let guess = b.len() / (n.max(1) * 8);
                for _ in 0..n {
                    let (name, after) = cstring(rest)?;
                    if after.len() < 18 {
                        return Err("a row description that ends early".into());
                    }
                    let oid = be32(&after[6..10]);
                    let format = be16(&after[16..18]);
                    if format != 0 {
                        return Err(format!("column '{}' is sent in the binary format, which is not read", name));
                    }
                    names.push(name.to_string());
                    oids.push(oid);
                    cols.push(col_of(oid, guess));
                    valids.push(Vec::with_capacity(guess));
                    rest = &after[18..];
                }
                described = true;
            }
            b'D' => {
                if !described {
                    return Err("rows without a row description".into());
                }
                if body.len() < 2 {
                    return Err("a row without its count".into());
                }
                let n = be16(body);
                if n != cols.len() {
                    return Err(format!("a row of {} cells for {} columns", n, cols.len()));
                }
                let mut p = 2;
                for c in 0..n {
                    if p + 4 > body.len() {
                        return Err("a row that ends early".into());
                    }
                    let l = be32(&body[p..]);
                    p += 4;
                    if l < 0 {
                        valids[c].push(false);
                        match &mut cols[c] {
                            Col::Int(v) => v.push(0),
                            Col::Float(v) => v.push(0.0),
                            Col::Str(v) => v.push(String::new()),
                            Col::Bool(v) => v.push(false),
                            Col::DateTime(v, z) => {
                                v.push(0);
                                z.push(0);
                            }
                        }
                        continue;
                    }
                    let l = l as usize;
                    if p + l > body.len() {
                        return Err("a row that ends early".into());
                    }
                    let cell = std::str::from_utf8(&body[p..p + l]).map_err(|_| format!("column '{}' holds text that is not UTF-8", names[c]))?;
                    p += l;
                    valids[c].push(true);
                    match &mut cols[c] {
                        Col::Int(v) => v.push(cell.parse().map_err(|_| format!("column '{}': cannot read '{}' as an int", names[c], cell))?),
                        Col::Float(v) => v.push(cell.parse().map_err(|_| format!("column '{}': cannot read '{}' as a float", names[c], cell))?),
                        Col::Bool(v) => v.push(cell == "t"),
                        Col::DateTime(v, z) => {
                            let (ms, offset) = crate::frame::parse_datetime(cell).ok_or_else(|| format!("column '{}': cannot read '{}' as a date or time", names[c], cell))?;
                            v.push(ms);
                            z.push(offset);
                        }
                        Col::Str(v) => v.push(cell.to_string()),
                    }
                }
            }
            b'E' => {
                // fields: a type byte and its text, to a zero byte
                let mut rest = body;
                let mut message = String::new();
                while let Some((&t, after)) = rest.split_first() {
                    if t == 0 {
                        break;
                    }
                    let (text, next) = cstring(after)?;
                    if t == b'M' {
                        message = text.to_string();
                    }
                    rest = next;
                }
                return Err(if message.is_empty() { "the server answered an error".to_string() } else { message });
            }
            b'Z' => {
                let data = cols
                    .into_iter()
                    .map(|c| match c {
                        Col::Int(v) => Column::Int(v),
                        Col::Float(v) => Column::Float(v),
                        Col::Str(v) => Column::Str(v),
                        Col::Bool(v) => Column::Bool(v),
                        Col::DateTime(v, z) => Column::DateTime(v, z),
                    })
                    .collect();
                return Ok(CrowFrame::new(names, data, valids));
            }
            // a command's completion, a notice, an empty query: nothing a frame holds
            _ => {}
        }
        i = end;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unhex(h: &str) -> Vec<u8> {
        (0..h.len()).step_by(2).map(|i| u8::from_str_radix(&h[i..i + 2], 16).unwrap()).collect()
    }

    const SAMPLE: &str = "540000010a000d6100000000000000000000170004ffffffff00006200000000000000000000170004ffffffff00006300000000000000000000140008ffffffff00006400000000000000000002bc0004ffffffff00006500000000000000000002bd0008ffffffff00006600000000000000000006a4ffffffffffff00006700000000000000000000100001ffffffff0000680000000000000000000019ffffffffffff0000690000000000000000000019ffffffffffff00006a000000000000000000045a0008ffffffff00006b00000000000000000004a00008ffffffff00006c000000000000000000043a0004ffffffff00006d00000000000000000003efffffffffffff00004400000092000d0000000137000000022d350000000a3930303030303030303000000003322e3500000004312e32350000000531322e33340000000174000000026869ffffffff00000017323032342d30312d30322030333a30343a30352e31323300000016323032342d30312d30312031393a30343a30352d30380000000a323032342d30312d3032000000057b312c327d440000007f000d00000001310000000132000000013300000003302e3500000004302e32350000000130000000016600000002796f000000017800000013323030302d30312d30312030303a30303a303000000016313939392d31322d33312031363a30303a30302d30380000000a323030302d30312d3031000000037b337d430000000d53454c4543542032005a0000000549";
    const ERROR: &str = "4500000067534552524f5200564552524f5200433432373033004d636f6c756d6e20226e6f70652220646f6573206e6f74206578697374005038004670617273655f72656c6174696f6e2e63004c3336353700526572726f724d697373696e67436f6c756d6e00005a0000000549";

    /// Two rows of every kind of column the scratch server answered —
    /// `SELECT 7::int2, -5::int4, 9000000000::int8, 2.5::float4,
    /// 1.25::float8, 12.34::numeric(10,2), true, 'hi', NULL::text,
    /// '2024-01-02 03:04:05.123'::timestamp, '2024-01-02 03:04:05+00'::timestamptz,
    /// '2024-01-02'::date, ARRAY[1,2]` and a second row — decoded as the
    /// frame holds them (the timestamptz came written in the server's
    /// zone, `2024-01-01 19:04:05-08`, and is the same moment)
    #[test]
    fn every_column_kind_decodes_as_the_frame_holds_it() {
        let f = frame_of_postgres(&unhex(SAMPLE)).unwrap();
        assert_eq!(f.columns, ["a", "b", "c", "d", "e", "f", "g", "h", "i", "j", "k", "l", "m"]);
        assert_eq!(f.nrows(), 2);
        let col = |name: &str| -> (&Column, &Vec<bool>) {
            let i = f.columns.iter().position(|c| c == name).unwrap();
            (&f.data[i], &f.validity[i])
        };
        assert!(matches!(col("a").0, Column::Int(v) if v == &[7, 1]));
        assert!(matches!(col("b").0, Column::Int(v) if v == &[-5, 2]));
        assert!(matches!(col("c").0, Column::Int(v) if v == &[9_000_000_000, 3]));
        assert!(matches!(col("d").0, Column::Float(v) if v == &[2.5, 0.5]));
        assert!(matches!(col("e").0, Column::Float(v) if v == &[1.25, 0.25]));
        assert!(matches!(col("f").0, Column::Float(v) if v == &[12.34, 0.0]));
        assert!(matches!(col("g").0, Column::Bool(v) if v == &[true, false]));
        assert!(matches!(col("h").0, Column::Str(v) if v == &["hi", "yo"]));
        assert!(matches!(col("i"), (Column::Str(v), valid) if v == &["", "x"] && valid == &[false, true]));
        assert!(matches!(col("j").0, Column::DateTime(v, _) if v == &[1_704_164_645_123, 946_684_800_000]));
        assert!(matches!(col("k").0, Column::DateTime(v, _) if v == &[1_704_164_645_000, 946_684_800_000]));
        assert!(matches!(col("l").0, Column::DateTime(v, _) if v == &[1_704_153_600_000, 946_684_800_000]));
        assert!(matches!(col("m").0, Column::Str(v) if v == &["{1,2}", "{3}"]));
    }

    /// The server's error is the reason in its words; bytes that end before
    /// the ready-for-query end early (a driver reads more); rows before a
    /// description, and a binary cell, are refused
    #[test]
    fn an_error_short_bytes_and_binary_cells_are_refused() {
        assert_eq!(frame_of_postgres(&unhex(ERROR)).unwrap_err(), "column \"nope\" does not exist");
        let whole = unhex(SAMPLE);
        // cut inside the ready-for-query (its six bytes start at 556), or just before it
        assert_eq!(frame_of_postgres(&whole[..whole.len() - 3]).unwrap_err(), "the answer ends early at byte 556");
        assert_eq!(frame_of_postgres(&whole[..whole.len() - 6]).unwrap_err(), "the answer ends early at byte 556");
        assert_eq!(frame_of_postgres(&[]).unwrap_err(), "the answer ends early at byte 0");
        // a row (of no cells) before any description
        let row_first = [b'D', 0, 0, 0, 6, 0, 0, b'Z', 0, 0, 0, 5, b'I'];
        assert_eq!(frame_of_postgres(&row_first).unwrap_err(), "rows without a row description");
        // a description of one column `n` of int4 in the binary format, then ready
        // 'T', its length 26, one column: "n\0", table oid 0, attnum 0, type oid 23, typlen 4, typmod -1, format 1
        let mut binary = vec![b'T', 0, 0, 0, 26, 0, 1, b'n', 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 23, 0, 4, 0xff, 0xff, 0xff, 0xff, 0, 1];
        binary.extend_from_slice(&[b'Z', 0, 0, 0, 5, b'I']);
        assert_eq!(frame_of_postgres(&binary).unwrap_err(), "column 'n' is sent in the binary format, which is not read");
        // a description alone is a frame with columns and no rows
        let mut text = binary.clone();
        text[26] = 0;
        let f = frame_of_postgres(&text).unwrap();
        assert_eq!((f.columns.clone(), f.nrows()), (vec!["n".to_string()], 0));
    }
}
