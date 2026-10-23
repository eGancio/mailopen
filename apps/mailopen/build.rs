//! On Windows, embed the app icon (drawn in code, `src/icon_art.rs`) and the version
//! information in `mailopen.exe`, so Explorer, the Start menu and the taskbar show them.

include!("src/icon_art.rs");

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/icon_art.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let Some(out) = std::env::var_os("OUT_DIR").map(std::path::PathBuf::from) else {
        println!("cargo:warning=OUT_DIR mancante: icona non inclusa");
        return;
    };
    let ico_path = out.join("mailopen.ico");
    if let Err(e) = std::fs::write(&ico_path, ico(&[16, 20, 24, 32, 40, 48, 64, 128, 256])) {
        println!("cargo:warning=icona non scritta: {e}");
        return;
    }
    let version = std::env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.1.0".into());
    let mut parts = version
        .split('.')
        .map(|p| p.parse::<u16>().unwrap_or(0))
        .chain(std::iter::repeat(0));
    let mut next = || parts.next().unwrap_or(0);
    let (a, b, c) = (next(), next(), next());
    let path = ico_path.display().to_string().replace('\\', "\\\\");
    let rc = format!(
        r#"1 ICON "{path}"
1 VERSIONINFO
FILEVERSION {a},{b},{c},0
PRODUCTVERSION {a},{b},{c},0
FILEOS 0x40004
FILETYPE 0x1
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "041004B0"
    BEGIN
      VALUE "CompanyName", "mailopen"
      VALUE "FileDescription", "mailopen - posta per tutti i tuoi account"
      VALUE "FileVersion", "{version}"
      VALUE "InternalName", "mailopen"
      VALUE "OriginalFilename", "mailopen.exe"
      VALUE "ProductName", "mailopen"
      VALUE "ProductVersion", "{version}"
      VALUE "LegalCopyright", "MIT OR Apache-2.0"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x410, 1200
  END
END
"#
    );
    let rc_path = out.join("mailopen.rc");
    if let Err(e) = std::fs::write(&rc_path, rc) {
        println!("cargo:warning=risorse non scritte: {e}");
        return;
    }
    if let Err(e) = embed_resource::compile(&rc_path, embed_resource::NONE).manifest_optional() {
        println!("cargo:warning=icona non inclusa nell'eseguibile: {e}");
    }
}
