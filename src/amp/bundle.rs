use std::{path::PathBuf, process::Stdio};

/// Discover patchable Amp bundles: CLI binary on PATH + editor extensions.
pub fn find_amp_bundles() -> Vec<PathBuf> {
    let mut seen = std::collections::HashSet::new();
    let mut result = Vec::new();

    // 1. Resolve `amp` from PATH.
    if let Ok(output) = std::process::Command::new("which")
        .arg("amp")
        .stderr(Stdio::null())
        .output()
        && let Ok(raw) = std::str::from_utf8(&output.stdout)
    {
        let raw = raw.trim();
        if !raw.is_empty()
            && let Ok(real) = std::fs::canonicalize(raw)
            // Skip native binaries (Mach-O / ELF) — only patch JS text files.
            && !is_native_binary(&real)
            && has_ad_code(&real)
            && seen.insert(real.clone())
        {
            result.push(real);
        }
    }

    // 2. VS Code / Cursor / Windsurf extensions.
    if let Ok(home) = byokey_daemon::paths::home_dir() {
        for editor_dir in &[".vscode", ".vscode-insiders", ".cursor", ".windsurf"] {
            let ext_root = home.join(editor_dir).join("extensions");
            if !ext_root.is_dir() {
                continue;
            }
            if let Ok(entries) = std::fs::read_dir(&ext_root) {
                for entry in entries.flatten() {
                    let name = entry.file_name();
                    let name_str = name.to_string_lossy();
                    if !name_str.starts_with("sourcegraph.amp-") {
                        continue;
                    }
                    if let Ok(walker) = glob_walk(&entry.path()) {
                        for js_file in walker {
                            if let Ok(meta) = js_file.metadata()
                                && meta.len() > 1_000_000
                                && let Ok(real) = std::fs::canonicalize(&js_file)
                                && has_ad_code(&real)
                                && seen.insert(real.clone())
                            {
                                result.push(real);
                            }
                        }
                    }
                }
            }
        }
    }

    result
}

/// Recursively yield `.js` files under `dir`.
fn glob_walk(dir: &std::path::Path) -> std::io::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            if let Ok(sub) = glob_walk(&path) {
                files.extend(sub);
            }
        } else if path.extension().is_some_and(|e| e == "js") {
            files.push(path);
        }
    }
    Ok(files)
}

/// Return `true` if the file starts with a Mach-O or ELF magic number.
fn is_native_binary(path: &std::path::Path) -> bool {
    use std::io::Read as _;
    let Ok(mut f) = std::fs::File::open(path) else {
        return false;
    };
    let mut magic = [0u8; 4];
    if f.read_exact(&mut magic).is_err() {
        return false;
    }
    let m = u32::from_be_bytes(magic);
    // Mach-O: fat (0xcafebabe), 64-bit LE (0xcffaedfe), 32-bit BE (0xfeedface),
    //         64-bit BE (0xfeedfacf), 32-bit LE (0xcefaedfe)
    matches!(
        m,
        0xcafe_babe | 0xcffa_edfe | 0xfeed_face | 0xfeed_facf | 0xcefa_edfe
    ) || magic == [0x7f, b'E', b'L', b'F'] // ELF
}

fn has_ad_code(path: &std::path::Path) -> bool {
    std::fs::read(path)
        .map(|data| {
            data.windows(b"fireImpressionIfNeeded".len())
                .any(|w| w == b"fireImpressionIfNeeded")
        })
        .unwrap_or(false)
}
