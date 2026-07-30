//! Install or remove a `mezon` CLI shim on the user PATH
//! - macOS: symlink in `~/.local/bin/mezon` and append PATH to `~/.zprofile`
//! - Linux: symlink in `~/.local/bin/mezon`
//! - Windows: symlink in `%LOCALAPPDATA%\\Mezon\\bin` and append that directory to the user PATH

use anyhow::Context as _;
use std::path::{Path, PathBuf};

const CLI_NAME: &str = if cfg!(windows) { "mezon.exe" } else { "mezon" };

#[cfg(target_os = "macos")]
const PATH_BLOCK_START: &str = "# >>> mezon-cli PATH >>>";
#[cfg(target_os = "macos")]
const PATH_BLOCK_END: &str = "# <<< mezon-cli PATH <<<";
#[cfg(target_os = "macos")]
const PATH_EXPORT_LINE: &str = r#"export PATH="$HOME/.local/bin:$PATH""#;

pub fn menu_visible() -> bool {
    #[cfg(target_os = "linux")]
    {
        if let Ok(exe) = std::env::current_exe()
            && let Some(parent) = exe.parent()
            && parent == Path::new("/usr/bin")
        {
            return false;
        }
    }
    true
}

pub fn is_installed() -> bool {
    let Some(link) = managed_link_path() else {
        return false;
    };
    if !link.exists() {
        return false;
    }
    points_to_current_exe(&link)
}

pub fn install() -> anyhow::Result<()> {
    let exe = std::env::current_exe().context("resolve current executable")?;
    let link = managed_link_path().context("resolve CLI install path")?;

    #[cfg(target_os = "windows")]
    return install_windows(&exe, &link);

    #[cfg(unix)]
    {
        install_unix_symlink(&exe, &link)?;
        #[cfg(target_os = "macos")]
        add_macos_shell_path_entry()?;
        Ok(())
    }

    #[cfg(not(any(unix, target_os = "windows")))]
    anyhow::bail!("CLI install is not supported on this platform")
}

pub fn uninstall() -> anyhow::Result<()> {
    let link = managed_link_path().context("resolve CLI install path")?;

    #[cfg(target_os = "windows")]
    remove_windows_path_entry()?;

    #[cfg(target_os = "macos")]
    remove_macos_shell_path_entry()?;

    if !link.exists() {
        return Ok(());
    }

    std::fs::remove_file(&link).with_context(|| format!("remove {}", link.display()))
}

pub fn toggle() -> anyhow::Result<bool> {
    if is_installed() {
        uninstall()?;
        Ok(false)
    } else {
        install()?;
        Ok(true)
    }
}

fn managed_link_path() -> Option<PathBuf> {
    Some(cli_bin_dir()?.join(CLI_NAME))
}

fn cli_bin_dir() -> Option<PathBuf> {
    #[cfg(unix)]
    {
        dirs::home_dir().map(|home| home.join(".local").join("bin"))
    }

    #[cfg(windows)]
    {
        dirs::data_local_dir().map(|dir| dir.join("Mezon").join("bin"))
    }

    #[cfg(not(any(unix, windows)))]
    {
        None
    }
}

#[cfg(unix)]
fn install_unix_symlink(exe: &Path, link: &Path) -> anyhow::Result<()> {
    if let Some(parent) = link.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    if link.exists() {
        std::fs::remove_file(link).with_context(|| format!("replace {}", link.display()))?;
    }
    std::os::unix::fs::symlink(exe, link)
        .with_context(|| format!("link {} -> {}", link.display(), exe.display()))?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn shell_profile_path() -> Option<PathBuf> {
    dirs::home_dir().map(|home| home.join(".zprofile"))
}

#[cfg(target_os = "macos")]
fn add_macos_shell_path_entry() -> anyhow::Result<()> {
    let profile = shell_profile_path().context("resolve home directory")?;
    let existing = read_profile(&profile)?;
    if profile_has_mezon_path_block(&existing) {
        return Ok(());
    }
    let mut content = existing;
    if !content.is_empty() && !content.ends_with('\n') {
        content.push('\n');
    }
    content.push_str(PATH_BLOCK_START);
    content.push('\n');
    content.push_str(PATH_EXPORT_LINE);
    content.push('\n');
    content.push_str(PATH_BLOCK_END);
    content.push('\n');
    write_profile(&profile, &content)
}

#[cfg(target_os = "macos")]
fn remove_macos_shell_path_entry() -> anyhow::Result<()> {
    let Some(profile) = shell_profile_path() else {
        return Ok(());
    };
    if !profile.exists() {
        return Ok(());
    }
    let existing = read_profile(&profile)?;
    let updated = remove_mezon_path_block(&existing);
    if updated == existing {
        return Ok(());
    }
    write_profile(&profile, &updated)
}

#[cfg(target_os = "macos")]
fn read_profile(path: &Path) -> anyhow::Result<String> {
    if path.exists() {
        std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))
    } else {
        Ok(String::new())
    }
}

#[cfg(target_os = "macos")]
fn write_profile(path: &Path, content: &str) -> anyhow::Result<()> {
    std::fs::write(path, content).with_context(|| format!("write {}", path.display()))
}

#[cfg(target_os = "macos")]
fn profile_has_mezon_path_block(content: &str) -> bool {
    content.contains(PATH_BLOCK_START)
}

#[cfg(target_os = "macos")]
fn remove_mezon_path_block(content: &str) -> String {
    let Some(start) = content.find(PATH_BLOCK_START) else {
        return content.to_string();
    };
    let search_from = start + PATH_BLOCK_START.len();
    let Some(end_rel) = content[search_from..].find(PATH_BLOCK_END) else {
        return content.to_string();
    };
    let end = search_from + end_rel + PATH_BLOCK_END.len();
    let mut updated = String::with_capacity(content.len());
    updated.push_str(&content[..start]);
    updated.push_str(&content[end..]);
    trim_trailing_blank_lines(&mut updated);
    updated
}

#[cfg(target_os = "macos")]
fn trim_trailing_blank_lines(content: &mut String) {
    while content.ends_with("\n\n") {
        content.pop();
    }
    if content == "\n" {
        content.clear();
    }
}

#[cfg(target_os = "windows")]
fn install_windows(exe: &Path, link: &Path) -> anyhow::Result<()> {
    if let Some(parent) = link.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    if link.exists() {
        std::fs::remove_file(link).ok();
    }
    std::os::windows::fs::symlink_file(exe, link).or_else(|_| {
        std::fs::copy(exe, link)
            .map(|_| ())
            .with_context(|| format!("copy {} to {}", exe.display(), link.display()))
    })?;
    add_windows_path_entry(link.parent().unwrap())
}

#[cfg(target_os = "windows")]
fn add_windows_path_entry(bin_dir: &Path) -> anyhow::Result<()> {
    use windows::Win32::System::Registry::{
        HKEY_CURRENT_USER, KEY_SET_VALUE, REG_SZ, RegCloseKey, RegOpenKeyExW, RegSetValueExW,
    };

    let bin = bin_dir
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("CLI bin path is not valid UTF-8"))?;
    let mut current = read_user_path()?;
    if path_list_contains(&current, bin) {
        return Ok(());
    }
    if !current.is_empty() {
        current.push(';');
    }
    current.push_str(bin);

    unsafe {
        let mut key = Default::default();
        let open_result = RegOpenKeyExW(
            HKEY_CURRENT_USER,
            windows::core::w!("Environment"),
            Some(0),
            KEY_SET_VALUE,
            &mut key,
        );
        if open_result != windows::Win32::Foundation::NO_ERROR {
            anyhow::bail!("open HKCU\\Environment failed: {}", open_result.0);
        }

        let wide_value: Vec<u16> = current.encode_utf16().chain(std::iter::once(0)).collect();
        let value_bytes: Vec<u8> = wide_value
            .iter()
            .flat_map(|word| word.to_le_bytes())
            .collect();
        let set_result = RegSetValueExW(
            key,
            windows::core::w!("Path"),
            Some(0),
            REG_SZ,
            Some(&value_bytes),
        );
        let _ = RegCloseKey(key);
        if set_result != windows::Win32::Foundation::NO_ERROR {
            anyhow::bail!("set user PATH failed: {}", set_result.0);
        }
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn remove_windows_path_entry() -> anyhow::Result<()> {
    use windows::Win32::System::Registry::{
        HKEY_CURRENT_USER, KEY_SET_VALUE, REG_SZ, RegCloseKey, RegOpenKeyExW, RegSetValueExW,
    };

    let Some(bin_dir) = cli_bin_dir() else {
        return Ok(());
    };
    let bin = bin_dir
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("CLI bin path is not valid UTF-8"))?;

    let mut current = read_user_path()?;
    let filtered = current
        .split(';')
        .filter(|entry| !entry.eq_ignore_ascii_case(bin))
        .collect::<Vec<_>>()
        .join(";");
    if filtered == current {
        return Ok(());
    }
    current = filtered;

    unsafe {
        let mut key = Default::default();
        let open_result = RegOpenKeyExW(
            HKEY_CURRENT_USER,
            windows::core::w!("Environment"),
            Some(0),
            KEY_SET_VALUE,
            &mut key,
        );
        if open_result != windows::Win32::Foundation::NO_ERROR {
            anyhow::bail!("open HKCU\\Environment failed: {}", open_result.0);
        }
        let wide_value: Vec<u16> = current.encode_utf16().chain(std::iter::once(0)).collect();
        let value_bytes: Vec<u8> = wide_value
            .iter()
            .flat_map(|word| word.to_le_bytes())
            .collect();
        let set_result = RegSetValueExW(
            key,
            windows::core::w!("Path"),
            Some(0),
            REG_SZ,
            Some(&value_bytes),
        );
        let _ = RegCloseKey(key);
        if set_result != windows::Win32::Foundation::NO_ERROR {
            anyhow::bail!("update user PATH failed: {}", set_result.0);
        }
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn read_user_path() -> anyhow::Result<String> {
    use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_SZ, RegGetValueW};

    unsafe {
        let mut buffer = vec![0u16; 32_768];
        let mut size = (buffer.len() * 2) as u32;
        let result = RegGetValueW(
            HKEY_CURRENT_USER,
            windows::core::w!("Environment"),
            windows::core::w!("Path"),
            RRF_RT_REG_SZ,
            None,
            Some(buffer.as_mut_ptr().cast()),
            Some(&mut size),
        );
        if result != windows::Win32::Foundation::NO_ERROR {
            anyhow::bail!("read user PATH failed: {}", result.0);
        }
        let len = (size as usize / 2).saturating_sub(1);
        Ok(String::from_utf16_lossy(&buffer[..len]))
    }
}

#[cfg(target_os = "windows")]
fn path_list_contains(path: &str, entry: &str) -> bool {
    path.split(';').any(|part| part.eq_ignore_ascii_case(entry))
}

fn points_to_current_exe(link: &Path) -> bool {
    let Ok(exe) = std::env::current_exe() else {
        return false;
    };
    let Ok(link_canon) = std::fs::canonicalize(link) else {
        return false;
    };
    let Ok(exe_canon) = std::fs::canonicalize(exe) else {
        return false;
    };
    link_canon == exe_canon
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_visible_is_true_on_macos() {
        #[cfg(target_os = "macos")]
        assert!(menu_visible());
    }

    #[cfg(windows)]
    #[test]
    fn path_list_contains_is_case_insensitive() {
        assert!(path_list_contains(
            r"C:\Apps;C:\Users\me\AppData\Local\Mezon\bin",
            r"C:\Users\me\AppData\Local\Mezon\bin"
        ));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_path_block_round_trip() {
        let original = "export FOO=bar\n";
        let with_block =
            format!("{original}\n{PATH_BLOCK_START}\n{PATH_EXPORT_LINE}\n{PATH_BLOCK_END}\n");
        assert!(profile_has_mezon_path_block(&with_block));
        let restored = remove_mezon_path_block(&with_block);
        assert_eq!(restored, original);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_path_block_remove_is_idempotent() {
        let content = "export FOO=bar\n";
        assert_eq!(remove_mezon_path_block(content), content);
    }
}
