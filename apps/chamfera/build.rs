//! Windows only: embeds the app icon and version information into `chamfera.exe`.
//!
//! The icon is generated from `assets/icons/chamfera.svg` at build time by
//! `packaging/windows/build-windows.ps1`, which passes its path in `CHAMFERA_WINDOWS_ICON`.
//! Without that variable the exe is built without an icon. On every other target this script
//! does nothing.

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=CHAMFERA_WINDOWS_ICON");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return Ok(());
    }
    let mut res = winresource::WindowsResource::new();
    match std::env::var("CHAMFERA_WINDOWS_ICON") {
        Ok(icon) if !icon.is_empty() => {
            if !std::path::Path::new(&icon).is_file() {
                return Err(format!("CHAMFERA_WINDOWS_ICON={icon}: no such file").into());
            }
            println!("cargo:rerun-if-changed={icon}");
            res.set_icon(&icon);
        }
        _ => println!(
            "cargo:warning=CHAMFERA_WINDOWS_ICON is not set: chamfera.exe is built without an icon"
        ),
    }
    res.set("ProductName", "Chamfera")
        .set("FileDescription", "Chamfera parametric 3D CAD");
    res.compile()?;
    Ok(())
}
