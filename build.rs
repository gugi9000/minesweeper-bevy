use std::{env, fs, path::PathBuf};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=gfx/makibishi-icon.png");
    if env::var("CARGO_CFG_TARGET_OS")? != "windows" {
        return Ok(());
    }

    // Windows ICO entries can contain the original PNG without resampling.
    let png = include_bytes!("gfx/makibishi-icon.png");
    let width = u32::from_be_bytes(png[16..20].try_into()?);
    let height = u32::from_be_bytes(png[20..24].try_into()?);
    if !(1..=256).contains(&width) || !(1..=256).contains(&height) {
        return Err("Windows app icon dimensions must be between 1 and 256 pixels".into());
    }
    let mut ico = vec![
        0,
        0,
        1,
        0,
        1,
        0,
        (width % 256) as u8,
        (height % 256) as u8,
        0,
        0,
    ];
    ico.extend_from_slice(&1u16.to_le_bytes());
    ico.extend_from_slice(&32u16.to_le_bytes());
    ico.extend_from_slice(&u32::try_from(png.len())?.to_le_bytes());
    ico.extend_from_slice(&22u32.to_le_bytes());
    ico.extend_from_slice(png);
    let path = PathBuf::from(env::var_os("OUT_DIR").ok_or("OUT_DIR is unavailable")?)
        .join("makibishi.ico");
    fs::write(&path, ico)?;
    winresource::WindowsResource::new()
        .set_icon(path.to_str().ok_or("Icon path is not valid UTF-8")?)
        .compile()?;
    Ok(())
}
