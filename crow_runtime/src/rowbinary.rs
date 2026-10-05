//! ClickHouse's `RowBinaryWithNamesAndTypes`, decoded straight into a
//! frame: the kernel a driver written in Crow hands the bytes an HTTP
//! query answers (`sys.frame_of_rowbinary`). A column is typed by the
//! header's type name — the integers an int column (a `UInt64` past
//! i64's range wraps, as the native driver's did), floats and decimals a
//! float column, `Bool` a bool column, dates and datetimes a datetime
//! column in UTC milliseconds, strings, enums (by name), UUIDs, IPs,
//! 128- and 256-bit integers and arrays (as text) a text column; a
//! `Nullable` cell that is NULL is a missing cell; `LowCardinality` is
//! its inner type. A type no frame cell can hold refuses the whole.
use crate::frame::{Column, CrowFrame};

/// A column being filled, by the kind its type names.
enum Col {
    Int(Vec<i64>),
    Float(Vec<f64>),
    Str(Vec<String>),
    Bool(Vec<bool>),
    DateTime(Vec<i64>),
}

#[derive(Debug, Clone)]
enum Ty {
    U8, I8, U16, I16, U32, I32, U64, I64, Wide(usize), F32, F64, Str, Fixed(usize),
    Date, Date32, DateTime, DateTime64(u32), Decimal(usize, u32), Bool,
    Enum8(Vec<(i64, String)>), Enum16(Vec<(i64, String)>), Uuid, Ipv4, Ipv6,
    Nullable(Box<Ty>), Array(Box<Ty>),
}

fn parse_type(s: &str) -> Result<Ty, String> {
    let s = s.trim();
    let inner = |prefix: &str| s.strip_prefix(prefix).and_then(|r| r.strip_suffix(')'));
    if let Some(t) = inner("LowCardinality(") { return parse_type(t); }
    if let Some(t) = inner("Nullable(") { return Ok(Ty::Nullable(Box::new(parse_type(t)?))); }
    if let Some(t) = inner("Array(") { return Ok(Ty::Array(Box::new(parse_type(t)?))); }
    if let Some(n) = inner("FixedString(") { return Ok(Ty::Fixed(n.trim().parse().map_err(|_| format!("FixedString size: {}", n))?)); }
    if s.starts_with("DateTime64(") {
        let args = inner("DateTime64(").unwrap_or("");
        let scale = args.split(',').next().unwrap_or("3").trim().parse().map_err(|_| format!("DateTime64 scale: {}", args))?;
        return Ok(Ty::DateTime64(scale));
    }
    if s == "DateTime" || s.starts_with("DateTime(") { return Ok(Ty::DateTime); }
    if let Some(args) = inner("Decimal(") {
        let mut it = args.split(',');
        let p: u32 = it.next().unwrap_or("").trim().parse().map_err(|_| format!("Decimal precision: {}", args))?;
        let sc: u32 = it.next().unwrap_or("").trim().parse().map_err(|_| format!("Decimal scale: {}", args))?;
        let width = if p <= 9 { 4 } else if p <= 18 { 8 } else if p <= 38 { 16 } else { 32 };
        return Ok(Ty::Decimal(width, sc));
    }
    for (name, width) in [("Decimal32(", 4), ("Decimal64(", 8), ("Decimal128(", 16), ("Decimal256(", 32)] {
        if let Some(sc) = inner(name) { return Ok(Ty::Decimal(width, sc.trim().parse().map_err(|_| format!("Decimal scale: {}", sc))?)); }
    }
    if s.starts_with("Enum8(") || s.starts_with("Enum16(") {
        let body = s.splitn(2, '(').nth(1).and_then(|r| r.strip_suffix(')')).unwrap_or("");
        let mut pairs = Vec::new();
        // 'name' = value, comma-separated; a name may hold a comma or an escaped quote
        let mut rest = body.trim_start();
        while let Some(r) = rest.strip_prefix('\'') {
            let mut name = String::new();
            let mut chars = r.char_indices();
            let mut end = 0;
            while let Some((i, c)) = chars.next() {
                if c == '\\' { if let Some((_, n)) = chars.next() { name.push(n); } continue; }
                if c == '\'' { end = i + 1; break; }
                name.push(c);
            }
            let after = r[end..].trim_start().strip_prefix('=').ok_or("Enum: '=' expected")?.trim_start();
            let num_end = after.find(',').unwrap_or(after.len());
            let value: i64 = after[..num_end].trim().parse().map_err(|_| format!("Enum value: {}", &after[..num_end]))?;
            pairs.push((value, name));
            rest = after[num_end..].trim_start().strip_prefix(',').unwrap_or("").trim_start();
        }
        return Ok(if s.starts_with("Enum8") { Ty::Enum8(pairs) } else { Ty::Enum16(pairs) });
    }
    Ok(match s {
        "UInt8" => Ty::U8, "Int8" => Ty::I8, "UInt16" => Ty::U16, "Int16" => Ty::I16,
        "UInt32" => Ty::U32, "Int32" => Ty::I32, "UInt64" => Ty::U64, "Int64" => Ty::I64,
        "UInt128" | "Int128" => Ty::Wide(16), "UInt256" | "Int256" => Ty::Wide(32),
        "Float32" => Ty::F32, "Float64" => Ty::F64, "String" => Ty::Str,
        "Date" => Ty::Date, "Date32" => Ty::Date32, "Bool" => Ty::Bool, "UUID" => Ty::Uuid,
        "IPv4" => Ty::Ipv4, "IPv6" => Ty::Ipv6,
        other => return Err(format!("a column of type {} is not a frame cell", other)),
    })
}

/// The frame column a type's cells fill.
fn col_of(t: &Ty, n: usize) -> Col {
    match t {
        Ty::Nullable(inner) => col_of(inner, n),
        Ty::U8 | Ty::I8 | Ty::U16 | Ty::I16 | Ty::U32 | Ty::I32 | Ty::U64 | Ty::I64 => Col::Int(Vec::with_capacity(n)),
        Ty::F32 | Ty::F64 | Ty::Decimal(..) => Col::Float(Vec::with_capacity(n)),
        Ty::Bool => Col::Bool(Vec::with_capacity(n)),
        Ty::Date | Ty::Date32 | Ty::DateTime | Ty::DateTime64(_) => Col::DateTime(Vec::with_capacity(n)),
        _ => Col::Str(Vec::with_capacity(n)),
    }
}

struct Reader<'a> { b: &'a [u8], i: usize }
impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let end = self.i.checked_add(n).filter(|&e| e <= self.b.len()).ok_or_else(|| format!("the rows end early at byte {}", self.i))?;
        let s = &self.b[self.i..end];
        self.i = end;
        Ok(s)
    }
    fn leb(&mut self) -> Result<usize, String> {
        let mut out: u64 = 0;
        let mut shift = 0;
        loop {
            let byte = self.take(1)?[0];
            out |= ((byte & 0x7f) as u64) << shift;
            if byte & 0x80 == 0 { return Ok(out as usize); }
            shift += 7;
            if shift > 63 { return Err("a length too long".into()); }
        }
    }
    fn str(&mut self) -> Result<&'a str, String> {
        let n = self.leb()?;
        std::str::from_utf8(self.take(n)?).map_err(|_| "text that is not UTF-8".to_string())
    }
    fn le<const N: usize>(&mut self) -> Result<[u8; N], String> {
        let s = self.take(N)?;
        let mut a = [0u8; N];
        a.copy_from_slice(s);
        Ok(a)
    }
}

fn push_cell(r: &mut Reader, t: &Ty, col: &mut Col, valid: &mut Vec<bool>) -> Result<(), String> {
    let (t, present) = match t {
        Ty::Nullable(inner) => (inner.as_ref(), r.take(1)?[0] == 0),
        t => (t, true),
    };
    valid.push(present);
    if !present {
        match col { Col::Int(v) => v.push(0), Col::Float(v) => v.push(0.0), Col::Str(v) => v.push(String::new()), Col::Bool(v) => v.push(false), Col::DateTime(v) => v.push(0) }
        return Ok(());
    }
    match (t, col) {
        (Ty::U8, Col::Int(v)) => v.push(r.take(1)?[0] as i64),
        (Ty::I8, Col::Int(v)) => v.push(r.take(1)?[0] as i8 as i64),
        (Ty::U16, Col::Int(v)) => v.push(u16::from_le_bytes(r.le()?) as i64),
        (Ty::I16, Col::Int(v)) => v.push(i16::from_le_bytes(r.le()?) as i64),
        (Ty::U32, Col::Int(v)) => v.push(u32::from_le_bytes(r.le()?) as i64),
        (Ty::I32, Col::Int(v)) => v.push(i32::from_le_bytes(r.le()?) as i64),
        (Ty::U64, Col::Int(v)) => v.push(u64::from_le_bytes(r.le()?) as i64),
        (Ty::I64, Col::Int(v)) => v.push(i64::from_le_bytes(r.le()?)),
        (Ty::Wide(n), Col::Str(v)) => { let bytes = r.take(*n)?; v.push(wide_text(bytes)) }
        (Ty::F32, Col::Float(v)) => v.push(f32::from_le_bytes(r.le()?) as f64),
        (Ty::F64, Col::Float(v)) => v.push(f64::from_le_bytes(r.le()?)),
        (Ty::Decimal(width, scale), Col::Float(v)) => {
            let raw: i128 = match width {
                4 => i32::from_le_bytes(r.le()?) as i128,
                8 => i64::from_le_bytes(r.le()?) as i128,
                16 => i128::from_le_bytes(r.le()?),
                // a 256-bit decimal's low 128 bits (its high bits are beyond a float's reach)
                _ => {
                    let low = i128::from_le_bytes(r.le()?);
                    r.take(16)?;
                    low
                }
            };
            v.push(raw as f64 / 10f64.powi(*scale as i32))
        }
        (Ty::Str, Col::Str(v)) => v.push(r.str()?.to_string()),
        (Ty::Fixed(n), Col::Str(v)) => { let b = r.take(*n)?; let end = b.iter().rposition(|&c| c != 0).map_or(0, |p| p + 1); v.push(String::from_utf8_lossy(&b[..end]).into_owned()) }
        (Ty::Date, Col::DateTime(v)) => v.push(u16::from_le_bytes(r.le()?) as i64 * 86_400_000),
        (Ty::Date32, Col::DateTime(v)) => v.push(i32::from_le_bytes(r.le()?) as i64 * 86_400_000),
        (Ty::DateTime, Col::DateTime(v)) => v.push(u32::from_le_bytes(r.le()?) as i64 * 1000),
        (Ty::DateTime64(scale), Col::DateTime(v)) => {
            let ticks = i64::from_le_bytes(r.le()?);
            let ms = if *scale >= 3 { ticks / 10i64.pow(scale - 3) } else { ticks * 10i64.pow(3 - scale) };
            v.push(ms)
        }
        (Ty::Bool, Col::Bool(v)) => v.push(r.take(1)?[0] != 0),
        (Ty::Enum8(names), Col::Str(v)) => { let k = r.take(1)?[0] as i8 as i64; v.push(names.iter().find(|(n, _)| *n == k).map(|(_, s)| s.clone()).unwrap_or_else(|| k.to_string())) }
        (Ty::Enum16(names), Col::Str(v)) => { let k = i16::from_le_bytes(r.le()?) as i64; v.push(names.iter().find(|(n, _)| *n == k).map(|(_, s)| s.clone()).unwrap_or_else(|| k.to_string())) }
        (Ty::Uuid, Col::Str(v)) => {
            let hi = u64::from_le_bytes(r.le()?);
            let lo = u64::from_le_bytes(r.le()?);
            let h = format!("{:032x}", ((hi as u128) << 64) | lo as u128);
            v.push(format!("{}-{}-{}-{}-{}", &h[..8], &h[8..12], &h[12..16], &h[16..20], &h[20..]))
        }
        (Ty::Ipv4, Col::Str(v)) => { let n = u32::from_le_bytes(r.le()?); v.push(std::net::Ipv4Addr::from(n).to_string()) }
        (Ty::Ipv6, Col::Str(v)) => { let b: [u8; 16] = r.le()?; v.push(std::net::Ipv6Addr::from(b).to_string()) }
        (Ty::Array(inner), Col::Str(v)) => {
            let n = r.leb()?;
            let mut items = Vec::with_capacity(n);
            for _ in 0..n { items.push(scalar_text(r, inner)?); }
            v.push(format!("[{}]", items.join(", ")))
        }
        (t, _) => return Err(format!("a column of type {:?} is not a frame cell", t)),
    }
    Ok(())
}

fn wide_text(bytes: &[u8]) -> String {
    let mut x: u128 = 0;
    for (k, &b) in bytes.iter().take(16).enumerate() { x |= (b as u128) << (8 * k); }
    x.to_string()
}

/// An array's item, as text: numbers and strings
fn scalar_text(r: &mut Reader, t: &Ty) -> Result<String, String> {
    let mut col = col_of(t, 1);
    let mut valid = Vec::new();
    push_cell(r, t, &mut col, &mut valid)?;
    if !valid[0] { return Ok("null".into()); }
    Ok(match col {
        Col::Int(v) => v[0].to_string(),
        Col::Float(v) => v[0].to_string(),
        Col::Str(v) => format!("'{}'", v[0]),
        Col::Bool(v) => v[0].to_string(),
        Col::DateTime(v) => v[0].to_string(),
    })
}

/// The frame the bytes hold, or why they do not.
pub fn frame_of_rowbinary(b: &[u8]) -> Result<CrowFrame, String> {
    let (names, cols, valids) = decode(b)?;
    let data = cols
        .into_iter()
        .map(|c| match c {
            Col::Int(v) => Column::Int(v),
            Col::Float(v) => Column::Float(v),
            Col::Str(v) => Column::Str(v),
            Col::Bool(v) => Column::Bool(v),
            Col::DateTime(v) => {
                let n = v.len();
                Column::DateTime(v, vec![0; n])
            }
        })
        .collect();
    Ok(CrowFrame::new(names, data, valids))
}

fn decode(b: &[u8]) -> Result<(Vec<String>, Vec<Col>, Vec<Vec<bool>>), String> {
    let mut r = Reader { b, i: 0 };
    let n = r.leb()?;
    let mut names = Vec::with_capacity(n);
    for _ in 0..n { names.push(r.str()?.to_string()); }
    let mut types = Vec::with_capacity(n);
    for _ in 0..n { types.push(parse_type(r.str()?)?); }
    let guess = b.len() / (n.max(1) * 8);
    let mut cols: Vec<Col> = types.iter().map(|t| col_of(t, guess)).collect();
    let mut valids: Vec<Vec<bool>> = (0..n).map(|_| Vec::with_capacity(guess)).collect();
    while r.i < b.len() {
        for c in 0..n {
            push_cell(&mut r, &types[c], &mut cols[c], &mut valids[c])?;
        }
    }
    Ok((names, cols, valids))
}


#[cfg(test)]
mod tests {
    use super::*;

    fn unhex(h: &str) -> Vec<u8> {
        (0..h.len()).step_by(2).map(|i| u8::from_str_radix(&h[i..i + 2], 16).unwrap()).collect()
    }

    /// Every kind of column ClickHouse answered for one row, decoded as
    /// the frame holds it: the server's own bytes (the query that made
    /// them is in the measurement's notes)
    #[test]
    fn every_column_kind_decodes_as_the_frame_holds_it() {
        let blob = unhex("12016101620163016401650166016701680169016a016b016c016d016e016f0170017101720555496e743805496e74363407466c6f6174363406537472696e670f4e756c6c61626c652855496e743829104e756c6c61626c6528537472696e672904446174650f4461746554696d6528275554432729144461746554696d65363428332c202755544327290e446563696d616c2831382c20322904426f6f6c17456e756d3828277827203d20312c20277927203d203229164c6f7743617264696e616c69747928537472696e672904555549440e4669786564537472696e672833290c41727261792855496e74382907466c6f617433320655496e74363401fbffffffffffffff00000000000004400268690003010c4d257d936503d920c88c010000d2040000000000000101026c63e711b35c04c4f061a0dbd36a00a67b906162000201020000c03fffffffffffffffff");
        let f = frame_of_rowbinary(&blob).unwrap();
        assert_eq!(f.columns, ["a", "b", "c", "d", "e", "f", "g", "h", "i", "j", "k", "l", "m", "n", "o", "p", "q", "r"]);
        let col = |name: &str| -> (&Column, &Vec<bool>) {
            let i = f.columns.iter().position(|c| c == name).unwrap();
            (&f.data[i], &f.validity[i])
        };
        assert!(matches!(col("a").0, Column::Int(v) if v == &[1]));
        assert!(matches!(col("b").0, Column::Int(v) if v == &[-5]));
        assert!(matches!(col("c").0, Column::Float(v) if v == &[2.5]));
        assert!(matches!(col("d").0, Column::Str(v) if v == &["hi"]));
        assert!(matches!(col("e"), (Column::Int(v), valid) if v == &[3] && valid == &[true]));
        assert!(matches!(col("f"), (Column::Str(v), valid) if v == &[""] && valid == &[false]));
        assert!(matches!(col("g").0, Column::DateTime(v, _) if v == &[1_704_153_600_000]));
        assert!(matches!(col("h").0, Column::DateTime(v, _) if v == &[1_704_164_645_000]));
        assert!(matches!(col("i").0, Column::DateTime(v, _) if v == &[1_704_164_645_123]));
        assert!(matches!(col("j").0, Column::Float(v) if (v[0] - 12.34).abs() < 1e-9));
        assert!(matches!(col("k").0, Column::Bool(v) if v == &[true]));
        assert!(matches!(col("l").0, Column::Str(v) if v == &["x"]));
        assert!(matches!(col("m").0, Column::Str(v) if v == &["lc"]));
        assert!(matches!(col("n").0, Column::Str(v) if v == &["61f0c404-5cb3-11e7-907b-a6006ad3dba0"]));
        assert!(matches!(col("o").0, Column::Str(v) if v == &["ab"]));
        assert!(matches!(col("p").0, Column::Str(v) if v == &["[1, 2]"]));
        assert!(matches!(col("q").0, Column::Float(v) if v == &[1.5]));
        // a UInt64 past i64's range wraps, as the native driver's did
        assert!(matches!(col("r").0, Column::Int(v) if v == &[-1]));
    }

    /// Bytes that end inside a row, and a type no cell holds, are refused
    /// with the reason
    #[test]
    fn short_bytes_and_a_type_no_cell_holds_are_refused() {
        let mut blob = unhex("12016101620163016401650166016701680169016a016b016c016d016e016f0170017101720555496e743805496e74363407466c6f6174363406537472696e670f4e756c6c61626c652855496e743829104e756c6c61626c6528537472696e672904446174650f4461746554696d6528275554432729144461746554696d65363428332c202755544327290e446563696d616c2831382c20322904426f6f6c17456e756d3828277827203d20312c20277927203d203229164c6f7743617264696e616c69747928537472696e672904555549440e4669786564537472696e672833290c41727261792855496e74382907466c6f617433320655496e74363401fbffffffffffffff00000000000004400268690003010c4d257d936503d920c88c010000d2040000000000000101026c63e711b35c04c4f061a0dbd36a00a67b906162000201020000c03fffffffffffffffff");
        blob.truncate(blob.len() - 3);
        assert_eq!(frame_of_rowbinary(&blob).unwrap_err(), "the rows end early at byte 330");
        // one column `t` of type Tuple(UInt8, UInt8)
        let tuple = [&[1u8, 1, b't', 19][..], b"Tuple(UInt8, UInt8)", &[1, 2]].concat();
        assert_eq!(frame_of_rowbinary(&tuple).unwrap_err(), "a column of type Tuple(UInt8, UInt8) is not a frame cell");
        assert_eq!(frame_of_rowbinary(&[]).unwrap_err(), "the rows end early at byte 0");
        // a header alone is a frame with columns and no rows
        let empty = [&[1u8, 1, b'n', 5][..], b"UInt8"].concat();
        let f = frame_of_rowbinary(&empty).unwrap();
        assert_eq!((f.columns.clone(), f.nrows()), (vec!["n".to_string()], 0));
    }
}
