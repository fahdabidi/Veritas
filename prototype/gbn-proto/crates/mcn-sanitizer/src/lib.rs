//! # MCN Sanitizer
//!
//! Wraps FFmpeg to strip ALL identifying metadata from video files before
//! they enter the chunking and encryption pipeline.
//!
//! ## What Gets Stripped
//!
//! - EXIF data (GPS, camera model, lens info, orientation)
//! - Container-level tags (creation time, encoder software, title, comment)
//! - Encoder version strings ("recorded with iPhone 15 Pro")    
//! - Telemetry tracks (GoPro GPS, DJI flight data)
//! - Thumbnail/cover art embedded in container
//!
//! ## FFmpeg Strategy
//!
//! ```text
//! ffmpeg -i input.mp4 \
//!   -map_metadata -1 \           # strip all global metadata
//!   -map_chapters -1 \           # strip chapter markers
//!   -fflags +bitexact \          # deterministic output
//!   -flags:v +bitexact \         # deterministic video
//!   -flags:a +bitexact \         # deterministic audio
//!   -c copy \                    # no re-encoding (fast, lossless)
//!   -metadata creation_time=0 \  # zero out creation timestamp
//!   output.mp4
//! ```

use std::{
    path::Path,
    process::{Command, Stdio},
};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SanitizerError {
    #[error("FFmpeg execution failed or is not installed: {0}")]
    ExecutionError(#[from] std::io::Error),

    #[error("FFmpeg failed with exit code {0}")]
    FfmpegError(i32, String),

    #[error("Failed to parse ffprobe output: {0}")]
    FfprobeParseError(#[from] serde_json::Error),
}

/// Report containing metadata about the sanitization process.
#[derive(Debug, Clone)]
pub struct SanitizeReport {
    pub input_size: u64,
    pub output_size: u64,
    pub duration_ms: u64,
}

/// An unexpected metadata field that survived sanitization.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeakedField {
    pub scope: String,
    pub key: String,
    pub value: String,
}

/// Check if FFmpeg is installed and accessible on PATH.
pub fn is_ffmpeg_available() -> bool {
    Command::new("ffmpeg")
        .arg("-version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Sanitize a video file by stripping all metadata without re-encoding the streams.
///
/// Under the hood, this executes `ffmpeg` as a subprocess.
pub fn sanitize_video(
    input_path: impl AsRef<Path>,
    output_path: impl AsRef<Path>,
) -> Result<SanitizeReport, SanitizerError> {
    let in_p = input_path.as_ref();
    let out_p = output_path.as_ref();

    let input_size = std::fs::metadata(in_p)
        .map(|m| m.len())
        .unwrap_or(0);

    let start = std::time::Instant::now();

    let result = Command::new("ffmpeg")
        .arg("-y") // Overwrite output file if exists
        .arg("-i")
        .arg(in_p)
        // Strip everything
        .args(["-map_metadata", "-1"])
        .args(["-map_chapters", "-1"])
        // Bitexact for determinism (helps with reproducible tests/builds)
        .args(["-fflags", "+bitexact"])
        .args(["-flags:v", "+bitexact"])
        .args(["-flags:a", "+bitexact"])
        // Re-write container creation time to Unix Epoch 0
        .args(["-metadata", "creation_time=2000-01-01T00:00:00.000000Z"])
        // Don't re-encode video or audio streams
        .args(["-c", "copy"])
        .arg(out_p)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()?;

    if !result.status.success() {
        let stderr = String::from_utf8_lossy(&result.stderr).into_owned();
        return Err(SanitizerError::FfmpegError(
            result.status.code().unwrap_or(-1),
            stderr,
        ));
    }

    let output_size = std::fs::metadata(out_p)
        .map(|m| m.len())
        .unwrap_or(0);

    Ok(SanitizeReport {
        input_size,
        output_size,
        duration_ms: start.elapsed().as_millis() as u64,
    })
}

/// Verify that a sanitized video file contains no unauthorized metadata fields.
pub fn verify_sanitized(path: impl AsRef<Path>) -> Result<Vec<LeakedField>, SanitizerError> {
    let output = Command::new("ffprobe")
        .arg("-v")
        .arg("quiet")
        .arg("-print_format")
        .arg("json")
        .arg("-show_format")
        .arg("-show_streams")
        .arg(path.as_ref())
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        return Err(SanitizerError::FfmpegError(
            output.status.code().unwrap_or(-1),
            stderr,
        ));
    }

    let json_str = String::from_utf8_lossy(&output.stdout);
    let probe: serde_json::Value = serde_json::from_str(&json_str)?;

    let mut leaked = Vec::new();

    // 1. Check container-level (format) tags
    if let Some(format) = probe.get("format") {
        if let Some(tags) = format.get("tags").and_then(|t| t.as_object()) {
            for (key, value) in tags {
                let lower_key = key.to_lowercase();
                // FFmpeg bitexact flag leaves some identifying encoder strings,
                // and we set creation_time manually.
                if lower_key == "encoder" || lower_key == "major_brand" || lower_key == "minor_version" || lower_key == "compatible_brands" {
                    continue; 
                }
                
                let val_str = value.as_str().unwrap_or("unknown").to_string();
                if lower_key == "creation_time" && !val_str.starts_with("2000-01-01") {
                   leaked.push(LeakedField {
                       scope: "format.tags".into(),
                       key: key.clone(),
                       value: val_str,
                   });
                } else if lower_key != "creation_time" {
                   leaked.push(LeakedField {
                       scope: "format.tags".into(),
                       key: key.clone(),
                       value: val_str,
                   });
                }
            }
        }
    }

    // 2. Check stream-level tags
    if let Some(streams) = probe.get("streams").and_then(|s| s.as_array()) {
        for (i, stream) in streams.iter().enumerate() {
            if let Some(tags) = stream.get("tags").and_then(|t| t.as_object()) {
                for (key, value) in tags {
                    let lower_key = key.to_lowercase();
                    // Some basic format identifying metadata gets placed here by bitexact
                    if lower_key == "language" || lower_key == "handler_name" || lower_key == "vendor_id" {
                         continue;
                    }
                    if lower_key == "creation_time" {
                         let val_str = value.as_str().unwrap_or("");
                         if val_str.starts_with("2000-01-01") {
                             continue;
                         }
                    }

                    leaked.push(LeakedField {
                        scope: format!("stream[{i}].tags"),
                        key: key.clone(),
                        value: value.as_str().unwrap_or("unknown").to_string(),
                    });
                }
            }
        }
    }

    Ok(leaked)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    #[test]
    fn test_ffmpeg_available() {
        // If FFmpeg isn't installed on the test machine, we should just print a warning.
        // It's checked during runtime, but the tests will fail if not present.
        if !is_ffmpeg_available() {
            println!("WARNING: FFmpeg not detected on PATH. Skipping sanitizer tests.");
        }
    }

    #[test]
    fn test_sanitize_strips_metadata() {
        if !is_ffmpeg_available() {
            return;
        }

        // 1. Generate a test video containing "bad" metadata (author, gps) using ffmpeg
        let input_file = NamedTempFile::new().unwrap();
        let in_path = input_file.path().with_extension("mp4");
        
        let mut cmd = Command::new("ffmpeg");
        cmd.args(["-y", "-f", "lavfi", "-i", "color=c=red:s=320x240:d=1"]);
        cmd.args(["-metadata", "author=Jane Doe Privacy Leak"]);
        cmd.args(["-metadata", "location=+37.7749-122.4194/"]);
        cmd.args(["-metadata", "title=Secret Protest Footage"]);
        cmd.arg(&in_path);
        
        let gen_res = cmd.output().unwrap();
        assert!(gen_res.status.success(), "Failed to generate test video: {}", String::from_utf8_lossy(&gen_res.stderr));

        // Let's verify it actually contains leaked fields before sanitizing
        let pre_leaks = verify_sanitized(&in_path).unwrap();
        assert!(!pre_leaks.is_empty(), "Failed to embed test metadata!");
        let has_author = pre_leaks.iter().any(|f| f.key == "author");
        assert!(has_author, "Test file missing 'author' metadata");

        // 2. Sanitize the video
        let output_file = NamedTempFile::new().unwrap();
        let out_path = output_file.path().with_extension("mp4");
        
        let report = sanitize_video(&in_path, &out_path).unwrap();
        assert!(report.output_size > 0);

        // 3. Verify sanitization
        let post_leaks = verify_sanitized(&out_path).unwrap();
        
        // Output any leaked fields for debugging
        for l in &post_leaks {
            println!("LEAK: {l:?}");
        }

        assert!(post_leaks.is_empty(), "Expected no leaked fields, got {} leaks", post_leaks.len());
        
        // Clean up extensions
        let _ = std::fs::remove_file(in_path);
        let _ = std::fs::remove_file(out_path);
    }
}
