use anyhow::Result;
use std::path::PathBuf;

const AMP_PATCH_MARKER: &[u8] = b"/*ampatch*";

/// Patch the ad widget `build()` to return a zero-height spacer.
/// Returns `Ok(Some(patched))` on success, `Ok(None)` if already patched.
pub fn amp_patch(data: &[u8]) -> Result<Option<Vec<u8>>> {
    if data
        .windows(AMP_PATCH_MARKER.len())
        .any(|w| w == AMP_PATCH_MARKER)
    {
        return Ok(None);
    }

    // 1. Find spacer widget constructor: `new <Name>({height:0})`
    let spacer_re = regex::bytes::Regex::new(r"new (\w+)\(\{height:0\}\)").expect("valid regex");
    let spacer_match = spacer_re.captures(data).ok_or_else(|| {
        anyhow::anyhow!("cannot find spacer widget pattern  new <X>({{height:0}})")
    })?;
    let spacer = &spacer_match[1];
    println!(
        "  spacer widget constructor: {}",
        std::str::from_utf8(spacer).unwrap_or("?")
    );

    // 2. Anchor on `fireImpressionIfNeeded(){`
    let anchor_re = regex::bytes::Regex::new(r"fireImpressionIfNeeded\(\)\{").expect("valid regex");
    let anchor = anchor_re
        .find(data)
        .ok_or_else(|| anyhow::anyhow!("cannot find fireImpressionIfNeeded(){{"))?;
    println!(
        "  anchor: fireImpressionIfNeeded() at byte {}",
        anchor.start()
    );

    let fire_body_end = find_brace_match(data, anchor.end() - 1)?;

    // 3. Locate `build(<arg>){` immediately after.
    let search_window = 300;
    let search_end = (fire_body_end + search_window).min(data.len());
    let build_re = regex::bytes::Regex::new(r"build\(\w{1,4}\)\{").expect("valid regex");
    let build_match = build_re
        .find(&data[fire_body_end..search_end])
        .ok_or_else(|| {
            anyhow::anyhow!(
                "cannot find build() within {search_window} bytes after \
                 fireImpressionIfNeeded (byte {fire_body_end})"
            )
        })?;

    let body_open = fire_body_end + build_match.end() - 1; // position of `{`
    let body_close = find_brace_match(data, body_open)?; // matching `}`
    let body_len = body_close - body_open - 1; // bytes between { and }

    println!(
        "  build() body: {body_len} bytes  [{}..{body_close})",
        body_open + 1
    );

    // 4. Build same-length replacement: `return new <Spacer>({height:0})/*ampatch*<pad>*/`
    let mut ret_stmt = Vec::new();
    ret_stmt.extend_from_slice(b"return new ");
    ret_stmt.extend_from_slice(spacer);
    ret_stmt.extend_from_slice(b"({height:0})");
    ret_stmt.extend_from_slice(AMP_PATCH_MARKER);

    let suffix = b"*/";
    let pad = body_len
        .checked_sub(ret_stmt.len() + suffix.len())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "replacement ({} bytes) exceeds original body ({body_len} bytes)",
                ret_stmt.len() + suffix.len()
            )
        })?;

    let mut replacement = ret_stmt;
    replacement.resize(replacement.len() + pad, b' ');
    replacement.extend_from_slice(suffix);
    assert_eq!(replacement.len(), body_len);

    // 5. Splice.
    let mut out = Vec::with_capacity(data.len());
    out.extend_from_slice(&data[..body_open + 1]);
    out.extend_from_slice(&replacement);
    out.extend_from_slice(&data[body_close..]);
    assert_eq!(out.len(), data.len(), "file length must not change");

    let preview_len = 80.min(replacement.len());
    let preview = String::from_utf8_lossy(&replacement[..preview_len]);
    let ellipsis = if replacement.len() > 80 { "..." } else { "" };
    println!("  injected: {preview}{ellipsis}");

    Ok(Some(out))
}

/// Match the `}` that balances the `{` at `start`, respecting string literals.
fn find_brace_match(data: &[u8], start: usize) -> Result<usize> {
    if data.get(start) != Some(&b'{') {
        anyhow::bail!("expected '{{' at byte {start}");
    }

    let mut depth: usize = 1;
    let mut pos = start + 1;
    let mut in_str: u8 = 0; // 0 = not in string; otherwise the quote char
    let mut esc = false;

    while pos < data.len() && depth > 0 {
        let b = data[pos];
        if esc {
            esc = false;
        } else if in_str != 0 {
            if b == b'\\' {
                esc = true;
            } else if b == in_str {
                in_str = 0;
            }
        } else {
            match b {
                b'"' | b'\'' | b'`' => in_str = b,
                b'{' => depth += 1,
                b'}' => depth -= 1,
                _ => {}
            }
        }
        pos += 1;
    }

    if depth != 0 {
        anyhow::bail!("unmatched brace starting at byte {start}");
    }
    Ok(pos - 1)
}

/// Restore a bundle from its `.bak` backup.
pub fn amp_restore(bundle_path: &std::path::Path) -> Result<()> {
    let bak = bundle_path.with_extension("js.bak");
    if !bak.exists() {
        // Try the Python-style `.bak` extension too (appended, not replaced).
        let bak_alt = PathBuf::from(format!("{}.bak", bundle_path.display()));
        if bak_alt.exists() {
            std::fs::copy(&bak_alt, bundle_path)?;
            println!("  restored from {}", bak_alt.display());
            return Ok(());
        }
        anyhow::bail!("no backup found at {}", bak.display());
    }
    std::fs::copy(&bak, bundle_path)?;
    println!("  restored from {}", bak.display());
    Ok(())
}

/// Ad-hoc re-sign a binary after patching (macOS only).
/// Silently skips on non-macOS or if `codesign` is unavailable.
#[cfg(target_os = "macos")]
pub fn resign_adhoc(path: &std::path::Path) {
    use std::process::Stdio;
    let status = std::process::Command::new("codesign")
        .args(["--sign", "-", "--force", "--preserve-metadata=entitlements"])
        .arg(path)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    match status {
        Ok(s) if s.success() => println!("  re-signed (ad-hoc)"),
        Ok(s) => eprintln!("  WARNING: codesign exited with {s}"),
        Err(e) => eprintln!("  WARNING: codesign not available: {e}"),
    }
}

#[cfg(not(target_os = "macos"))]
pub fn resign_adhoc(_path: &std::path::Path) {}
