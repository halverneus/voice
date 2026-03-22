use anyhow::{anyhow, Result};
use std::path::Path;

/// Download a file at `url` to `dest`, calling `progress` with (bytes_done, total_bytes).
/// Runs synchronously — call from a dedicated thread.
pub fn download<F>(url: &str, dest: &Path, mut progress: F) -> Result<()>
where
    F: FnMut(u64, Option<u64>),
{
    use std::io::Write;

    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let client = reqwest::blocking::Client::builder()
        .timeout(None) // Models can be large; no hard timeout
        .build()?;

    let response = client.get(url).send()?;
    if !response.status().is_success() {
        return Err(anyhow!("HTTP {}: {}", response.status(), url));
    }

    let total = response.content_length();
    let mut downloaded: u64 = 0;

    // Write to a .part file first, rename on success
    let part_path = dest.with_extension("bin.part");
    let mut file = std::fs::File::create(&part_path)?;

    // reqwest::blocking::Response implements Read
    let mut reader = response;
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        use std::io::Read;
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])?;
        downloaded += n as u64;
        progress(downloaded, total);
    }

    file.flush()?;
    drop(file);
    std::fs::rename(&part_path, dest)?;
    Ok(())
}
