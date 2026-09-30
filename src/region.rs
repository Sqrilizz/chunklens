use std::{fs, io::Read, path::Path};

use anyhow::{Context, Result, anyhow, bail, ensure};
use flate2::bufread::{GzDecoder, ZlibDecoder};

pub const SECTOR: usize = 4096;
const MAX_CHUNK_BYTES: usize = 64 * 1024 * 1024;

pub fn region_coordinates(path: &Path) -> Option<(i32, i32)> {
    let name = path.file_name()?.to_str()?;
    let fields: Vec<_> = name.strip_suffix(".mca")?.split('.').collect();
    (fields.len() == 3 && fields[0] == "r")
        .then(|| Some((fields[1].parse().ok()?, fields[2].parse().ok()?)))
        .flatten()
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
    use super::{decompress, region_coordinates};
    use flate2::{
        Compression,
        write::{GzEncoder, ZlibEncoder},
    };
    use std::io::Write;
    use std::path::Path;
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
}
