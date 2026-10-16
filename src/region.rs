use std::{
    fs,
    io::{Read, Seek, SeekFrom},
    path::Path,
};

use anyhow::{Context, Result, anyhow, bail, ensure};
use flate2::bufread::{GzDecoder, ZlibDecoder};

pub const SECTOR: usize = 4096;
const MAX_CHUNK_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChunkSlotHeader {
    pub slot: usize,
    pub sector_offset: usize,
    pub sector_count: usize,
    pub timestamp: u32,
}

#[allow(dead_code)]
pub const LINEAR_SIGNATURE: u64 = 0xC3FF_1318_3CCA_9D9A;
pub const LINEAR_SIGNATURE_BYTES: [u8; 8] = [0xC3, 0xFF, 0x13, 0x18, 0x3C, 0xCA, 0x9D, 0x9A];
pub const LINEAR_HEADER_SIZE: usize = 32;
pub const LINEAR_FOOTER_SIZE: usize = 8;

pub fn region_coordinates(path: &Path) -> Option<(i32, i32)> {
    let name = path.file_name()?.to_str()?;
    let stem = name
        .strip_suffix(".mca")
        .or_else(|| name.strip_suffix(".linear"))?;
    let fields: Vec<_> = stem.split('.').collect();
    (fields.len() == 3 && fields[0] == "r")
        .then(|| Some((fields[1].parse().ok()?, fields[2].parse().ok()?)))
        .flatten()
}

pub fn read_region_header(path: &Path) -> Result<Vec<ChunkSlotHeader>> {
    let mut file =
        std::fs::File::open(path).with_context(|| format!("reading {}", path.display()))?;
    let mut magic = [0u8; 8];
    let n = file.read(&mut magic)?;
    if n >= 8 && magic == LINEAR_SIGNATURE_BYTES {
        return read_linear_header(path);
    }
    file.seek(SeekFrom::Start(0))?;
    let mut header = [0u8; SECTOR * 2];
    file.read_exact(&mut header)
        .with_context(|| format!("reading header from {}", path.display()))?;

    let mut slots = Vec::with_capacity(1024);
    for index in 0..1024 {
        let loc = index * 4;
        let offset = ((header[loc] as usize) << 16)
            | ((header[loc + 1] as usize) << 8)
            | header[loc + 2] as usize;
        let sectors = header[loc + 3] as usize;
        let ts_loc = 4096 + index * 4;
        let timestamp = u32::from_be_bytes(header[ts_loc..ts_loc + 4].try_into().unwrap());
        if offset >= 2 && sectors > 0 {
            slots.push(ChunkSlotHeader {
                slot: index,
                sector_offset: offset,
                sector_count: sectors,
                timestamp,
            });
        }
    }
    Ok(slots)
}

fn read_linear_header(path: &Path) -> Result<Vec<ChunkSlotHeader>> {
    let mut file =
        std::fs::File::open(path).with_context(|| format!("reading {}", path.display()))?;
    file.seek(SeekFrom::Start(LINEAR_HEADER_SIZE as u64))?;
    let mut decoder =
        zstd::Decoder::new(file).context("initializing zstd decoder for linear file header")?;
    let mut table = [0u8; 8192];
    decoder
        .read_exact(&mut table)
        .context("reading 8192-byte chunk table from linear stream")?;

    let mut slots = Vec::with_capacity(1024);
    for slot in 0..1024 {
        let entry = slot * 8;
        let chunk_size = u32::from_be_bytes(table[entry..entry + 4].try_into().unwrap()) as usize;
        let timestamp = u32::from_be_bytes(table[entry + 4..entry + 8].try_into().unwrap());
        if chunk_size > 0 {
            slots.push(ChunkSlotHeader {
                slot,
                sector_offset: chunk_size,
                sector_count: 1,
                timestamp,
            });
        }
    }
    Ok(slots)
}

fn read_single_chunk_linear(path: &Path, slot: usize, decompress_buf: &mut Vec<u8>) -> Result<()> {
    if slot >= 1024 {
        bail!("chunk slot {slot} out of bounds (max 1023)");
    }
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    if bytes.len() < LINEAR_HEADER_SIZE + LINEAR_FOOTER_SIZE {
        bail!("linear region file is shorter than 40 bytes");
    }
    let compressed_len = u32::from_be_bytes(bytes[20..24].try_into().unwrap()) as usize;
    let payload_end = if compressed_len > 0 && LINEAR_HEADER_SIZE + compressed_len <= bytes.len() {
        LINEAR_HEADER_SIZE + compressed_len
    } else {
        bytes.len().saturating_sub(LINEAR_FOOTER_SIZE)
    };
    if payload_end <= LINEAR_HEADER_SIZE {
        bail!("empty linear compressed payload");
    }
    let compressed_slice = &bytes[LINEAR_HEADER_SIZE..payload_end];

    let mut decompressed = Vec::new();
    let mut decoder = zstd::Decoder::new(compressed_slice)
        .context("initializing zstd decoder for linear file")?;
    decoder
        .read_to_end(&mut decompressed)
        .context("decompressing zstd stream for linear file")?;

    if decompressed.len() < 8192 {
        bail!("decompressed linear stream too short for chunk table");
    }

    let mut payload_offset = 8192;
    for s in 0..1024 {
        let entry = s * 8;
        let chunk_size =
            u32::from_be_bytes(decompressed[entry..entry + 4].try_into().unwrap()) as usize;
        if s == slot {
            if chunk_size == 0 {
                bail!("chunk at slot {slot} is not generated");
            }
            if payload_offset + chunk_size > decompressed.len() {
                bail!("chunk payload exceeds decompressed linear stream");
            }
            decompress_buf.clear();
            decompress_buf
                .extend_from_slice(&decompressed[payload_offset..payload_offset + chunk_size]);
            return Ok(());
        }
        payload_offset += chunk_size;
    }

    bail!("chunk at slot {slot} not found")
}

pub fn read_single_chunk(
    path: &Path,
    slot: usize,
    sector_offset: usize,
    sectors: usize,
    decompress_buf: &mut Vec<u8>,
    decompressor: &mut libdeflater::Decompressor,
) -> Result<()> {
    let mut magic = [0u8; 8];
    {
        let mut file =
            std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
        let _ = file.read(&mut magic);
    }
    if magic == LINEAR_SIGNATURE_BYTES {
        return read_single_chunk_linear(path, slot, decompress_buf);
    }

    let mut file =
        std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let location = sector_offset
        .checked_mul(SECTOR)
        .ok_or_else(|| anyhow!("invalid sector offset"))?;
    file.seek(SeekFrom::Start(location as u64))
        .with_context(|| format!("seeking in {}", path.display()))?;

    let mut length_bytes = [0u8; 4];
    file.read_exact(&mut length_bytes)?;
    let length = u32::from_be_bytes(length_bytes) as usize;
    if length == 0 || length + 4 > sectors * SECTOR {
        bail!("invalid chunk length or sector allocation in slot {slot}");
    }

    let mut compression_byte = [0u8; 1];
    file.read_exact(&mut compression_byte)?;
    let compression = compression_byte[0];

    if compression & 0x80 != 0 {
        let (region_x, region_z) = region_coordinates(path).context("invalid region filename")?;
        let x = region_x
            .checked_mul(32)
            .and_then(|n| n.checked_add((slot % 32) as i32))
            .context("external chunk X coordinate overflow")?;
        let z = region_z
            .checked_mul(32)
            .and_then(|n| n.checked_add((slot / 32) as i32))
            .context("external chunk Z coordinate overflow")?;
        let ext_path = path
            .parent()
            .context("region path has no parent")?
            .join(format!("c.{x}.{z}.mcc"));
        let ext_payload = fs::read(&ext_path)
            .with_context(|| format!("reading external {}", ext_path.display()))?;
        decompress_into_with(decompressor, compression, &ext_payload, decompress_buf)
            .context("decompressing external chunk")?;
    } else {
        let payload_len = length.saturating_sub(1);
        let mut raw_slice = vec![0u8; payload_len];
        file.read_exact(&mut raw_slice)?;
        decompress_into_with(decompressor, compression, &raw_slice, decompress_buf)
            .context("decompressing chunk payload")?;
    }
    Ok(())
}

#[allow(dead_code)]
pub fn chunks_in_region(path: &Path) -> Result<Vec<(usize, Result<Vec<u8>>)>> {
    let mut result = Vec::new();
    let mut buf = Vec::with_capacity(256 * 1024);
    for_each_chunk(path, &mut buf, |slot, res| {
        result.push((slot, res.map(|d| d.to_vec())));
    })?;
    Ok(result)
}

pub fn for_each_chunk<F>(path: &Path, decompress_buf: &mut Vec<u8>, callback: F) -> Result<()>
where
    F: FnMut(usize, Result<&[u8]>),
{
    let mut file_buf = Vec::new();
    for_each_chunk_with_buffers(path, &mut file_buf, decompress_buf, callback)
}

pub fn for_each_chunk_with_buffers<F>(
    path: &Path,
    file_buf: &mut Vec<u8>,
    decompress_buf: &mut Vec<u8>,
    callback: F,
) -> Result<()>
where
    F: FnMut(usize, Result<&[u8]>),
{
    let mut decompressor = libdeflater::Decompressor::new();
    for_each_chunk_with_buffers_and_decompressor(
        path,
        file_buf,
        decompress_buf,
        &mut decompressor,
        callback,
    )
}

pub fn for_each_chunk_with_buffers_and_decompressor<F>(
    path: &Path,
    file_buf: &mut Vec<u8>,
    decompress_buf: &mut Vec<u8>,
    decompressor: &mut libdeflater::Decompressor,
    mut callback: F,
) -> Result<()>
where
    F: FnMut(usize, Result<&[u8]>),
{
    file_buf.clear();
    let mut file =
        std::fs::File::open(path).with_context(|| format!("reading {}", path.display()))?;
    file.read_to_end(file_buf)
        .with_context(|| format!("reading {}", path.display()))?;
    let bytes = file_buf.as_slice();

    if bytes.len() >= 8 && bytes[..8] == LINEAR_SIGNATURE_BYTES {
        return for_each_chunk_linear(bytes, decompress_buf, callback);
    }

    if bytes.len() < SECTOR * 2 {
        bail!("region header is shorter than 8192 bytes");
    }

    for index in 0..1024 {
        let start = index * 4;
        let offset = ((bytes[start] as usize) << 16)
            | ((bytes[start + 1] as usize) << 8)
            | bytes[start + 2] as usize;
        let sectors = bytes[start + 3] as usize;
        if offset == 0 && sectors == 0 {
            continue;
        }
        if offset < 2 || sectors == 0 {
            callback(index, Err(anyhow!("invalid chunk sector allocation")));
            continue;
        }
        let location = offset
            .checked_mul(SECTOR)
            .ok_or_else(|| anyhow!("invalid sector offset"))?;
        if location + 5 > bytes.len() {
            callback(
                index,
                Err(anyhow!("chunk data offset is outside region file")),
            );
            continue;
        }
        let length_bytes: [u8; 4] = match bytes[location..location + 4].try_into() {
            Ok(b) => b,
            Err(_) => {
                callback(index, Err(anyhow!("invalid chunk length")));
                continue;
            }
        };
        let length = u32::from_be_bytes(length_bytes) as usize;
        if length == 0 || length + 4 > sectors * SECTOR || location + length + 4 > bytes.len() {
            callback(
                index,
                Err(anyhow!("invalid chunk length or sector allocation")),
            );
            continue;
        }
        let compression = bytes[location + 4];

        if compression & 0x80 != 0 {
            let (region_x, region_z) = match region_coordinates(path) {
                Some(c) => c,
                None => {
                    callback(index, Err(anyhow!("invalid region filename")));
                    continue;
                }
            };
            let Some(x) = region_x
                .checked_mul(32)
                .and_then(|n| n.checked_add((index % 32) as i32))
            else {
                callback(index, Err(anyhow!("external chunk X coordinate overflow")));
                continue;
            };
            let Some(z) = region_z
                .checked_mul(32)
                .and_then(|n| n.checked_add((index / 32) as i32))
            else {
                callback(index, Err(anyhow!("external chunk Z coordinate overflow")));
                continue;
            };
            let ext_path = match path.parent() {
                Some(parent) => parent.join(format!("c.{x}.{z}.mcc")),
                None => {
                    callback(index, Err(anyhow!("region path has no parent")));
                    continue;
                }
            };
            match fs::read(&ext_path) {
                Ok(ext_payload) => {
                    decompress_buf.clear();
                    match decompress_into_with(
                        decompressor,
                        compression,
                        &ext_payload,
                        decompress_buf,
                    ) {
                        Ok(()) => callback(index, Ok(decompress_buf.as_slice())),
                        Err(e) => callback(index, Err(e.context(format!("chunk slot {index}")))),
                    }
                }
                Err(e) => {
                    callback(index, Err(anyhow!("external chunk payload: {e}")));
                }
            }
        } else {
            let raw_slice = &bytes[location + 5..location + 4 + length];
            decompress_buf.clear();
            match decompress_into_with(decompressor, compression, raw_slice, decompress_buf) {
                Ok(()) => callback(index, Ok(decompress_buf.as_slice())),
                Err(e) => callback(index, Err(e.context(format!("chunk slot {index}")))),
            }
        }
    }
    Ok(())
}

fn for_each_chunk_linear<F>(
    bytes: &[u8],
    decompress_buf: &mut Vec<u8>,
    mut callback: F,
) -> Result<()>
where
    F: FnMut(usize, Result<&[u8]>),
{
    if bytes.len() < LINEAR_HEADER_SIZE + LINEAR_FOOTER_SIZE {
        bail!("linear region file is shorter than 40 bytes");
    }
    if bytes[..8] != LINEAR_SIGNATURE_BYTES {
        bail!("invalid linear signature at header");
    }
    let version = bytes[8];
    if version != 1 && version != 2 {
        bail!("unsupported linear format version: {version}");
    }
    let compressed_len = u32::from_be_bytes(bytes[20..24].try_into().unwrap()) as usize;
    let payload_end = if compressed_len > 0 && LINEAR_HEADER_SIZE + compressed_len <= bytes.len() {
        LINEAR_HEADER_SIZE + compressed_len
    } else {
        bytes.len().saturating_sub(LINEAR_FOOTER_SIZE)
    };
    if payload_end <= LINEAR_HEADER_SIZE {
        bail!("empty linear compressed payload");
    }
    let compressed_slice = &bytes[LINEAR_HEADER_SIZE..payload_end];

    decompress_buf.clear();
    let mut decoder = zstd::Decoder::new(compressed_slice)
        .context("initializing zstd decoder for linear file")?;
    decoder
        .read_to_end(decompress_buf)
        .context("decompressing zstd stream for linear file")?;

    let decompressed = decompress_buf.as_slice();
    if decompressed.len() < 8192 {
        bail!("decompressed linear stream is smaller than 8192-byte chunk table");
    }

    let mut payload_offset = 8192;
    for slot in 0..1024 {
        let entry_offset = slot * 8;
        let chunk_size = u32::from_be_bytes(
            decompressed[entry_offset..entry_offset + 4]
                .try_into()
                .unwrap(),
        ) as usize;
        let _timestamp = u32::from_be_bytes(
            decompressed[entry_offset + 4..entry_offset + 8]
                .try_into()
                .unwrap(),
        );

        if chunk_size == 0 {
            continue;
        }

        if payload_offset + chunk_size > decompressed.len() {
            callback(
                slot,
                Err(anyhow!(
                    "chunk {slot} payload of size {chunk_size} exceeds decompressed stream length {}",
                    decompressed.len()
                )),
            );
            break;
        }

        let chunk_slice = &decompressed[payload_offset..payload_offset + chunk_size];
        callback(slot, Ok(chunk_slice));
        payload_offset += chunk_size;
    }

    Ok(())
}

pub fn decompress_into(compression: u8, payload: &[u8], output: &mut Vec<u8>) -> Result<()> {
    let mut decompressor = libdeflater::Decompressor::new();
    decompress_into_with(&mut decompressor, compression, payload, output)
}

pub fn decompress_into_with(
    decompressor: &mut libdeflater::Decompressor,
    compression: u8,
    payload: &[u8],
    output: &mut Vec<u8>,
) -> Result<()> {
    output.clear();
    match compression & 0x7f {
        1 => inflate_with(
            payload,
            output,
            |p, out| decompressor.gzip_decompress(p, out),
            |p, out| {
                GzDecoder::new(p)
                    .take(MAX_CHUNK_BYTES as u64 + 1)
                    .read_to_end(out)?;
                Ok(())
            },
        )?,
        2 => inflate_with(
            payload,
            output,
            |p, out| decompressor.zlib_decompress(p, out),
            |p, out| {
                ZlibDecoder::new(p)
                    .take(MAX_CHUNK_BYTES as u64 + 1)
                    .read_to_end(out)?;
                Ok(())
            },
        )?,
        3 => {
            ensure!(
                payload.len() <= MAX_CHUNK_BYTES,
                "chunk exceeds 64 MiB decompression limit"
            );
            output.extend_from_slice(payload);
        }
        4 => decompress_java_lz4(payload, output)?,
        kind => bail!("unsupported compression type {kind}"),
    }
    ensure!(
        output.len() <= MAX_CHUNK_BYTES,
        "chunk exceeds 64 MiB decompression limit"
    );
    Ok(())
}

fn inflate_with<F, R>(
    payload: &[u8],
    output: &mut Vec<u8>,
    mut decompress: F,
    fallback: R,
) -> Result<()>
where
    F: FnMut(&[u8], &mut [u8]) -> std::result::Result<usize, libdeflater::DecompressionError>,
    R: FnOnce(&[u8], &mut Vec<u8>) -> Result<()>,
{
    let mut target_len = (payload.len() * 6).clamp(64 * 1024, MAX_CHUNK_BYTES);
    loop {
        if output.len() < target_len {
            output.resize(target_len, 0);
        }
        match decompress(payload, &mut output[..target_len]) {
            Ok(written) => {
                output.truncate(written);
                return Ok(());
            }
            Err(libdeflater::DecompressionError::InsufficientSpace) => {
                if target_len >= MAX_CHUNK_BYTES {
                    bail!("chunk exceeds 64 MiB decompression limit");
                }
                target_len = (target_len * 2).min(MAX_CHUNK_BYTES);
            }
            Err(_) => {
                output.clear();
                return fallback(payload, output);
            }
        }
    }
}

fn decompress_java_lz4(mut input: &[u8], output: &mut Vec<u8>) -> Result<()> {
    loop {
        ensure!(
            input.len() >= 21 && &input[..8] == b"LZ4Block",
            "invalid or truncated LZ4Block header"
        );
        let method = input[8] & 0xf0;
        let block_limit = 1_usize << (10 + (input[8] & 0x0f));
        let compressed_len = u32::from_le_bytes(input[9..13].try_into()?) as usize;
        let raw_len = u32::from_le_bytes(input[13..17].try_into()?) as usize;
        let checksum = u32::from_le_bytes(input[17..21].try_into()?);
        input = &input[21..];
        ensure!(matches!(method, 0x10 | 0x20), "unknown LZ4Block method");
        if compressed_len == 0 && raw_len == 0 {
            ensure!(
                checksum == 0 && input.is_empty(),
                "invalid LZ4Block terminator"
            );
            return Ok(());
        }
        ensure!(
            raw_len > 0
                && raw_len <= block_limit
                && compressed_len > 0
                && compressed_len <= input.len(),
            "invalid LZ4Block length"
        );
        let start = output.len();
        ensure!(
            raw_len <= MAX_CHUNK_BYTES.saturating_sub(start),
            "chunk exceeds 64 MiB decompression limit"
        );
        let (block, rest) = input.split_at(compressed_len);
        input = rest;
        if method == 0x10 {
            ensure!(raw_len == compressed_len, "invalid raw LZ4Block length");
            output.extend_from_slice(block);
        } else {
            output.resize(start + raw_len, 0);
            let written = lz4_flex::block::decompress_into(block, &mut output[start..])
                .context("LZ4Block payload")?;
            ensure!(written == raw_len, "LZ4Block output length mismatch");
        }
        let actual = xxhash_rust::xxh32::xxh32(&output[start..], 0x9747b28c) & 0x0fff_ffff;
        ensure!(actual == checksum, "LZ4Block checksum mismatch");
    }
}

#[allow(dead_code)]
fn decompress(compression: u8, payload: &[u8]) -> Result<Vec<u8>> {
    let mut output = Vec::with_capacity(64 * 1024);
    decompress_into(compression, payload, &mut output)?;
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::{
        ChunkSlotHeader, decompress, read_region_header, read_single_chunk, region_coordinates,
    };
    use super::{LINEAR_SIGNATURE_BYTES, for_each_chunk};
    use flate2::{
        Compression,
        write::{GzEncoder, ZlibEncoder},
    };
    use std::io::Write;
    use std::path::Path;

    #[test]
    fn parses_linear_region_coordinates() {
        assert_eq!(
            region_coordinates(Path::new("r.-5.12.linear")),
            Some((-5, 12))
        );
        assert_eq!(region_coordinates(Path::new("bad.linear")), None);
    }

    #[test]
    fn parses_linear_region_file() {
        let mut uncompressed = Vec::with_capacity(8192 + 100);
        let mut table = [0u8; 8192];
        let chunk0_data = b"hello chunk 0 nbt payload";
        let chunk5_data = b"world chunk 5 nbt payload";

        // slot 0
        table[0..4].copy_from_slice(&(chunk0_data.len() as u32).to_be_bytes());
        table[4..8].copy_from_slice(&1000u32.to_be_bytes());

        // slot 5
        let off5 = 5 * 8;
        table[off5..off5 + 4].copy_from_slice(&(chunk5_data.len() as u32).to_be_bytes());
        table[off5 + 4..off5 + 8].copy_from_slice(&2000u32.to_be_bytes());

        uncompressed.extend_from_slice(&table);
        uncompressed.extend_from_slice(chunk0_data);
        uncompressed.extend_from_slice(chunk5_data);

        let compressed = zstd::encode_all(&uncompressed[..], 3).unwrap();

        let mut file_bytes = Vec::new();
        // 32-byte header
        file_bytes.extend_from_slice(&LINEAR_SIGNATURE_BYTES); // 8 bytes
        file_bytes.push(1); // version 1
        file_bytes.extend_from_slice(&2000u64.to_be_bytes()); // newest timestamp (8 bytes)
        file_bytes.push(3); // compression level (1 byte)
        file_bytes.extend_from_slice(&2u16.to_be_bytes()); // chunk count (2 bytes)
        file_bytes.extend_from_slice(&(compressed.len() as u32).to_be_bytes()); // data length (4 bytes)
        file_bytes.extend_from_slice(&[0u8; 8]); // reserved (8 bytes)
        assert_eq!(file_bytes.len(), 32);

        // payload
        file_bytes.extend_from_slice(&compressed);

        // 8-byte footer
        file_bytes.extend_from_slice(&LINEAR_SIGNATURE_BYTES);

        let temp = tempfile::tempdir().unwrap();
        let linear_path = temp.path().join("r.0.0.linear");
        std::fs::write(&linear_path, &file_bytes).unwrap();

        // 1. Test read_region_header
        let headers = read_region_header(&linear_path).unwrap();
        assert_eq!(headers.len(), 2);
        assert_eq!(headers[0].slot, 0);
        assert_eq!(headers[0].timestamp, 1000);
        assert_eq!(headers[1].slot, 5);
        assert_eq!(headers[1].timestamp, 2000);

        // 2. Test for_each_chunk
        let mut chunks_read = Vec::new();
        let mut decompress_buf = Vec::new();
        for_each_chunk(&linear_path, &mut decompress_buf, |slot, res| {
            chunks_read.push((slot, res.unwrap().to_vec()));
        })
        .unwrap();
        assert_eq!(chunks_read.len(), 2);
        assert_eq!(chunks_read[0].0, 0);
        assert_eq!(chunks_read[0].1, chunk0_data);
        assert_eq!(chunks_read[1].0, 5);
        assert_eq!(chunks_read[1].1, chunk5_data);

        // 3. Test read_single_chunk
        let mut out = Vec::new();
        let mut decompressor = libdeflater::Decompressor::new();
        read_single_chunk(&linear_path, 0, 0, 0, &mut out, &mut decompressor).unwrap();
        assert_eq!(out, chunk0_data);

        read_single_chunk(&linear_path, 5, 0, 0, &mut out, &mut decompressor).unwrap();
        assert_eq!(out, chunk5_data);
    }

    #[test]
    fn parses_region_coordinates() {
        assert_eq!(
            region_coordinates(Path::new("r.-18.41.mca")),
            Some((-18, 41))
        );
        assert_eq!(region_coordinates(Path::new("bad.mca")), None);
    }
    #[test]
    fn supports_all_mvp_compression_types() {
        let data = b"chunklens";
        let mut gzip = GzEncoder::new(Vec::new(), Compression::default());
        gzip.write_all(data).expect("gzip write");
        let mut zlib = ZlibEncoder::new(Vec::new(), Compression::default());
        zlib.write_all(data).expect("zlib write");
        assert_eq!(
            decompress(1, &gzip.finish().expect("gzip finish")).expect("gzip"),
            data
        );
        assert_eq!(
            decompress(2, &zlib.finish().expect("zlib finish")).expect("zlib"),
            data
        );
        assert_eq!(decompress(3, data).expect("raw"), data);
        assert!(decompress(5, data).is_err());
    }

    fn lz4_block(data: &[u8], compressed: bool) -> Vec<u8> {
        let payload = if compressed {
            lz4_flex::block::compress(data)
        } else {
            data.to_vec()
        };
        let mut bytes = b"LZ4Block".to_vec();
        bytes.push(if compressed { 0x26 } else { 0x16 });
        bytes.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&(data.len() as u32).to_le_bytes());
        bytes.extend_from_slice(
            &(xxhash_rust::xxh32::xxh32(data, 0x9747b28c) & 0x0fff_ffff).to_le_bytes(),
        );
        bytes.extend(payload);
        bytes
    }

    #[test]
    fn reads_java_lz4_blocks_and_checks_integrity() {
        let data = vec![42; 4096];
        let mut bytes = lz4_block(&data, true);
        bytes.extend(lz4_block(b"tail", false));
        bytes.extend_from_slice(b"LZ4Block\x16\0\0\0\0\0\0\0\0\0\0\0\0");
        let mut expected = data;
        expected.extend_from_slice(b"tail");
        assert_eq!(decompress(4, &bytes).unwrap(), expected);
        assert!(decompress(4, &bytes[..bytes.len() - 1]).is_err());
        bytes[17] ^= 1;
        assert!(decompress(4, &bytes).is_err());
    }

    #[test]
    fn reads_external_payload_without_modifying_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("r.-1.0.mca");
        let mut bytes = vec![0; super::SECTOR * 3];
        bytes[..4].copy_from_slice(&[0, 0, 2, 1]);
        bytes[8192..8196].copy_from_slice(&1_u32.to_be_bytes());
        bytes[8196] = 0x83;
        std::fs::write(&path, &bytes).unwrap();
        std::fs::write(dir.path().join("c.-32.0.mcc"), b"external").unwrap();
        let chunks = super::chunks_in_region(&path).unwrap();
        assert_eq!(chunks[0].1.as_ref().unwrap(), b"external");
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
    }
    #[test]
    fn reads_independent_java_compressor_fixture() {
        let bytes = include_bytes!("../tests/fixtures/lz4-java.bin");
        assert_eq!(decompress(4, bytes).unwrap(), b"chunklens".repeat(20_000));
    }

    #[test]
    fn reads_header_and_single_chunk_via_seek() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("r.0.0.mca");
        let mut bytes = vec![0; super::SECTOR * 3];
        // slot 0 at sector 2, length 1 sector, timestamp 12345
        bytes[..4].copy_from_slice(&[0, 0, 2, 1]);
        bytes[4096..4100].copy_from_slice(&12345_u32.to_be_bytes());

        // payload at sector 2 (8192 bytes offset)
        let chunk_data = b"hello seek chunk";
        let mut zlib = ZlibEncoder::new(Vec::new(), Compression::default());
        zlib.write_all(chunk_data).unwrap();
        let compressed = zlib.finish().unwrap();

        let len = (compressed.len() + 1) as u32;
        bytes[8192..8196].copy_from_slice(&len.to_be_bytes());
        bytes[8196] = 2; // zlib
        bytes[8197..8197 + compressed.len()].copy_from_slice(&compressed);

        std::fs::write(&path, &bytes).unwrap();

        let headers = read_region_header(&path).unwrap();
        assert_eq!(headers.len(), 1);
        assert_eq!(
            headers[0],
            ChunkSlotHeader {
                slot: 0,
                sector_offset: 2,
                sector_count: 1,
                timestamp: 12345,
            }
        );

        let mut out = Vec::new();
        let mut decompressor = libdeflater::Decompressor::new();
        read_single_chunk(&path, 0, 2, 1, &mut out, &mut decompressor).unwrap();
        assert_eq!(out, chunk_data);
    }
}
