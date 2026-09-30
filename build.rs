//! Windows 실행 파일에 아이콘과 버전 정보를 넣는다. 다른 대상에서는 아무것도 하지 않는다.
//! macOS 에서 크로스 빌드할 때는 mingw 의 windres 를 쓴다 (brew install mingw-w64).
fn main() {
    println!("cargo:rerun-if-changed=assets/icon/hwp2pdf.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut res = winresource::WindowsResource::new();
    if !cfg!(windows) {
        res.set_windres_path("x86_64-w64-mingw32-windres");
        res.set_ar_path("x86_64-w64-mingw32-ar");
    }
    res.set_icon("assets/icon/hwp2pdf.ico");
    res.set("FileDescription", "HWP2PDF - HWP/HWPX 변환기");
    res.set("ProductName", "HWP2PDF");
    if let Err(e) = res.compile() {
        println!("cargo:warning=아이콘 리소스를 넣지 못했습니다: {e}");
    }
}
