//! Windows 실행 파일에 아이콘과 버전 정보를 넣는다. 다른 OS 에서는 아무것도 하지 않는다.
fn main() {
    println!("cargo:rerun-if-changed=assets/icon/hwp2pdf.ico");
    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon/hwp2pdf.ico");
        res.set("FileDescription", "HWP2PDF - HWP/HWPX 변환기");
        res.set("ProductName", "HWP2PDF");
        if let Err(e) = res.compile() {
            println!("cargo:warning=아이콘 리소스를 넣지 못했습니다: {e}");
        }
    }
}
