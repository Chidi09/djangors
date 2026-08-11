use super::*;

#[test]
fn parses_a_clean_response() {
    assert_eq!(parse_response(b"stream: OK\0").unwrap(), ScanResult::Clean);
}

#[test]
fn parses_an_infected_response_and_extracts_the_signature_name() {
    assert_eq!(
        parse_response(b"stream: Eicar-Test-Signature FOUND\0").unwrap(),
        ScanResult::Infected("Eicar-Test-Signature".to_string())
    );
}

#[test]
fn rejects_a_response_that_does_not_look_like_clamd_at_all() {
    assert!(matches!(
        parse_response(b"not a clamd response"),
        Err(ClamAvError::UnexpectedResponse(_))
    ));
}

// The remaining tests talk to a real clamd over its real Unix socket - they
// are the actual proof this client's wire-protocol implementation works,
// not just that this file's own parsing logic is self-consistent. Skipped
// (not failed) if no clamd is reachable at the standard Debian/Ubuntu
// socket path, since CI/most dev machines won't have one running.
const REAL_CLAMD_SOCKET: &str = "/var/run/clamav/clamd.ctl";

#[tokio::test]
async fn real_clamd_flags_the_standard_eicar_test_string() {
    if !std::path::Path::new(REAL_CLAMD_SOCKET).exists() {
        eprintln!("skipping: no clamd socket at {REAL_CLAMD_SOCKET}");
        return;
    }
    let scanner = ClamAvScanner::unix(REAL_CLAMD_SOCKET);
    // The standard, harmless EICAR antivirus test string - every real AV
    // engine is required to flag this exact byte sequence, and nothing else
    // should ever match it.
    let eicar = b"X5O!P%@AP[4\\PZX54(P^)7CC)7}$EICAR-STANDARD-ANTIVIRUS-TEST-FILE!$H+H*";
    let result = scanner.scan(eicar).await.unwrap();
    assert_eq!(
        result,
        ScanResult::Infected("Eicar-Test-Signature".to_string())
    );
}

#[tokio::test]
async fn real_clamd_passes_an_ordinary_file() {
    if !std::path::Path::new(REAL_CLAMD_SOCKET).exists() {
        eprintln!("skipping: no clamd socket at {REAL_CLAMD_SOCKET}");
        return;
    }
    let scanner = ClamAvScanner::unix(REAL_CLAMD_SOCKET);
    let result = scanner.scan(b"just a normal harmless file").await.unwrap();
    assert_eq!(result, ScanResult::Clean);
}

#[tokio::test]
async fn real_clamd_flags_eicar_even_when_split_across_many_small_chunks() {
    if !std::path::Path::new(REAL_CLAMD_SOCKET).exists() {
        eprintln!("skipping: no clamd socket at {REAL_CLAMD_SOCKET}");
        return;
    }
    // A 4-byte chunk size forces the EICAR signature to be split across many
    // INSTREAM frames - proves chunking is real and clamd reassembles the
    // stream correctly, not just that a single whole-buffer write happens
    // to work.
    let scanner = ClamAvScanner {
        addr: ClamdAddr::Unix(REAL_CLAMD_SOCKET.into()),
        chunk_size: 4,
    };
    let eicar = b"X5O!P%@AP[4\\PZX54(P^)7CC)7}$EICAR-STANDARD-ANTIVIRUS-TEST-FILE!$H+H*";
    let result = scanner.scan(eicar).await.unwrap();
    assert_eq!(
        result,
        ScanResult::Infected("Eicar-Test-Signature".to_string())
    );
}
