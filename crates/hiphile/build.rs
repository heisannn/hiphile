//! Embeds the icon in the Windows executable, for Explorer, the taskbar and GPUI's window class,
//! which loads icon resource 1.

fn main() {
    println!("cargo:rerun-if-changed=../../assets/icon.rc");
    println!("cargo:rerun-if-changed=../../assets/icon.ico");
    embed_resource::compile(
        "../../assets/icon.rc",
        embed_resource::ParamsIncludeDirs(["../../assets"]),
    )
    .manifest_required()
    .unwrap();
}
