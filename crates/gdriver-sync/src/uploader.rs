//! Upload-related types and constants.

/// Files below this size use simple multipart upload.
pub const RESUMABLE_THRESHOLD: u64 = 5 * 1024 * 1024; // 5 MiB

/// Chunk size for resumable uploads (must be a multiple of 256 KiB per the
/// Google Drive API specification).
pub const CHUNK_SIZE: u64 = 5 * 1024 * 1024; // 5 MiB

/// Minimum chunk size required by Google Drive resumable upload protocol.
pub const MIN_CHUNK_SIZE: u64 = 256 * 1024; // 256 KiB

/// Maximum number of retry attempts before logging a persistent error.
pub const MAX_RETRIES: i32 = 3;

/// Validate that a chunk size conforms to Google's requirements.
///
/// Must be a multiple of 256 KiB.  Returns `true` when valid.
pub fn is_valid_chunk_size(size: u64) -> bool {
    size > 0 && size % MIN_CHUNK_SIZE == 0
}

/// Returns the recommended chunk size for a file of the given length.
/// Small files use a single chunk; large files use [`CHUNK_SIZE`].
pub fn recommended_chunk_size(file_len: u64) -> u64 {
    if file_len <= MIN_CHUNK_SIZE {
        file_len
    } else {
        CHUNK_SIZE
    }
}

/// Calculate the number of chunks needed for a file of `file_len` bytes
/// using `chunk_size` bytes per chunk.
pub fn chunk_count(file_len: u64, chunk_size: u64) -> u64 {
    if file_len == 0 {
        0
    } else {
        (file_len + chunk_size - 1) / chunk_size
    }
}

/// Calculate the byte range for chunk index `n` (0-based) of a file of
/// `file_len` bytes with `chunk_size` bytes per chunk.
///
/// Returns `(start, end)` where `end` is inclusive.  Returns `None` when
/// `n` is out of range.
pub fn chunk_range(n: u64, file_len: u64, chunk_size: u64) -> Option<(u64, u64)> {
    if file_len == 0 {
        return None;
    }
    let start = n * chunk_size;
    if start >= file_len {
        return None;
    }
    let end = std::cmp::min(start + chunk_size, file_len) - 1;
    Some((start, end))
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Constants ──────────────────────────────────────────────────────────

    #[test]
    fn resumable_threshold_is_5_mib() {
        assert_eq!(RESUMABLE_THRESHOLD, 5_242_880);
    }

    #[test]
    fn chunk_size_is_multiple_of_min() {
        assert_eq!(CHUNK_SIZE % MIN_CHUNK_SIZE, 0);
    }

    #[test]
    fn min_chunk_size_is_256_kib() {
        assert_eq!(MIN_CHUNK_SIZE, 262_144);
    }

    #[test]
    fn max_retries_is_positive() {
        assert!(MAX_RETRIES > 0);
    }

    // ── Chunk size validation ──────────────────────────────────────────────

    #[test]
    fn valid_chunk_sizes() {
        assert!(is_valid_chunk_size(256 * 1024));           // 256 KiB
        assert!(is_valid_chunk_size(512 * 1024));           // 512 KiB
        assert!(is_valid_chunk_size(1024 * 1024));          // 1 MiB
        assert!(is_valid_chunk_size(5 * 1024 * 1024));      // 5 MiB
        assert!(is_valid_chunk_size(256 * 1024 * 1024));    // 256 MiB
    }

    #[test]
    fn invalid_chunk_sizes() {
        assert!(!is_valid_chunk_size(0));
        assert!(!is_valid_chunk_size(1));
        assert!(!is_valid_chunk_size(100));
        assert!(!is_valid_chunk_size(256 * 1024 + 1));      // 256 KiB + 1
        assert!(!is_valid_chunk_size(256 * 1024 - 1));      // 256 KiB - 1
        assert!(!is_valid_chunk_size(1000 * 1024));         // not multiple of 256 KiB
    }

    // ── Recommended chunk size ─────────────────────────────────────────────

    #[test]
    fn small_file_uses_whole_file() {
        assert_eq!(recommended_chunk_size(100), 100);
        assert_eq!(recommended_chunk_size(MIN_CHUNK_SIZE), MIN_CHUNK_SIZE);
    }

    #[test]
    fn large_file_uses_5_mib_chunks() {
        assert_eq!(recommended_chunk_size(10 * 1024 * 1024), CHUNK_SIZE);
        assert_eq!(recommended_chunk_size(100 * 1024 * 1024), CHUNK_SIZE);
    }

    // ── Chunk count ────────────────────────────────────────────────────────

    #[test]
    fn chunk_count_zero_file() {
        assert_eq!(chunk_count(0, CHUNK_SIZE), 0);
    }

    #[test]
    fn chunk_count_one_chunk() {
        assert_eq!(chunk_count(100, CHUNK_SIZE), 1);
        assert_eq!(chunk_count(CHUNK_SIZE, CHUNK_SIZE), 1);
    }

    #[test]
    fn chunk_count_exact_multiple() {
        assert_eq!(chunk_count(2 * CHUNK_SIZE, CHUNK_SIZE), 2);
        assert_eq!(chunk_count(10 * CHUNK_SIZE, CHUNK_SIZE), 10);
    }

    #[test]
    fn chunk_count_remainder() {
        assert_eq!(chunk_count(CHUNK_SIZE + 1, CHUNK_SIZE), 2);
        assert_eq!(chunk_count(CHUNK_SIZE + CHUNK_SIZE - 1, CHUNK_SIZE), 2);
        assert_eq!(chunk_count(2 * CHUNK_SIZE + 1, CHUNK_SIZE), 3);
    }

    #[test]
    fn chunk_count_one_byte() {
        assert_eq!(chunk_count(1, CHUNK_SIZE), 1);
    }

    // ── Chunk range ────────────────────────────────────────────────────────

    #[test]
    fn chunk_range_zero_file() {
        assert_eq!(chunk_range(0, 0, CHUNK_SIZE), None);
    }

    #[test]
    fn chunk_range_first_chunk() {
        assert_eq!(chunk_range(0, 100, CHUNK_SIZE), Some((0, 99)));
        assert_eq!(chunk_range(0, CHUNK_SIZE, CHUNK_SIZE), Some((0, CHUNK_SIZE - 1)));
    }

    #[test]
    fn chunk_range_second_chunk() {
        assert_eq!(
            chunk_range(1, 2 * CHUNK_SIZE, CHUNK_SIZE),
            Some((CHUNK_SIZE, 2 * CHUNK_SIZE - 1))
        );
    }

    #[test]
    fn chunk_range_last_partial_chunk() {
        let file_len = CHUNK_SIZE + 100;
        assert_eq!(
            chunk_range(1, file_len, CHUNK_SIZE),
            Some((CHUNK_SIZE, file_len - 1))
        );
    }

    #[test]
    fn chunk_range_out_of_bounds() {
        assert_eq!(chunk_range(1, 100, CHUNK_SIZE), None);
        assert_eq!(chunk_range(5, CHUNK_SIZE, CHUNK_SIZE), None);
        assert_eq!(chunk_range(2, 2 * CHUNK_SIZE, CHUNK_SIZE), None);
    }

    #[test]
    fn chunk_range_single_byte() {
        assert_eq!(chunk_range(0, 1, CHUNK_SIZE), Some((0, 0)));
        assert_eq!(chunk_range(1, 1, CHUNK_SIZE), None);
    }

    #[test]
    fn chunk_range_exact_fit() {
        // File is exactly one chunk.
        assert_eq!(
            chunk_range(0, CHUNK_SIZE, CHUNK_SIZE),
            Some((0, CHUNK_SIZE - 1))
        );
    }
}
