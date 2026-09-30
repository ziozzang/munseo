// 릴리스 빌드의 Windows 에서는 콘솔 창 없이 GUI 로 뜬다.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod batch;
mod convert;
mod docx;
mod equation;
mod fonts;
mod gui;
mod icc;
mod latex;
mod omml;
mod pdfa;

use std::path::PathBuf;
use std::process::ExitCode;

use convert::OutputFormat;

fn main() -> ExitCode {
    // Finder 가 붙이는 -psn_ 인자는 무시한다.
    let args: Vec<String> = std::env::args().skip(1).filter(|a| !a.starts_with("-psn_")).collect();
    if args.is_empty() {
        return match gui::run() {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("GUI 실행 실패: {e}");
                ExitCode::FAILURE
            }
        };
    }
    attach_console();
    run_cli(&args)
}

fn print_usage() {
    eprintln!(
        "사용법: hwp2pdf [--format pdfa|pdf|docx|md|txt|html] [--delete-original] <파일|폴더>...\n\
         인자 없이 실행하면 드래그앤드롭 창이 열립니다.\n\
         --format            출력 포맷 (기본값: pdfa = PDF/A-2u)\n\
         --delete-original   변환에 성공한 원본 HWP 를 휴지통으로 옮김"
    );
}

fn run_cli(args: &[String]) -> ExitCode {
    let mut format = OutputFormat::PdfA;
    let mut delete_original = false;
    let mut inputs = Vec::new();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                print_usage();
                return ExitCode::SUCCESS;
            }
            "-f" | "--format" => {
                let Some(f) = it.next().and_then(|v| OutputFormat::parse(v)) else {
                    eprintln!("--format 값이 올바르지 않습니다 (pdfa|pdf|docx|md|txt|html)");
                    return ExitCode::from(2);
                };
                format = f;
            }
            "--delete-original" => delete_original = true,
            other if other.starts_with('-') => {
                eprintln!("알 수 없는 옵션: {other}");
                print_usage();
                return ExitCode::from(2);
            }
            other => inputs.push(PathBuf::from(other)),
        }
    }

    let files = batch::collect(&inputs);
    if files.is_empty() {
        eprintln!("변환할 HWP/HWPX 파일이 없습니다.");
        return ExitCode::from(1);
    }

    let mut failed = 0;
    for file in &files {
        match batch::process(file, format, delete_original, &|_, _| {}) {
            batch::Outcome::Ok { output, pages, original_deleted, warnings } => {
                let note = if original_deleted { " (원본 삭제됨)" } else { "" };
                println!("OK   {} → {} ({pages}쪽){note}", file.display(), output.display());
                for w in warnings {
                    println!("     주의: {w}");
                }
            }
            batch::Outcome::Failed(e) => {
                failed += 1;
                eprintln!("FAIL {}: {e}", file.display());
            }
        }
    }
    println!("완료: {}개 성공, {failed}개 실패", files.len() - failed);
    if failed == 0 { ExitCode::SUCCESS } else { ExitCode::FAILURE }
}

/// windows_subsystem = "windows" 로 빌드해도 터미널에서 CLI 로 부르면 출력이 보이게 한다.
#[cfg(windows)]
fn attach_console() {
    extern "system" {
        fn AttachConsole(pid: u32) -> i32;
    }
    const ATTACH_PARENT_PROCESS: u32 = u32::MAX;
    unsafe {
        AttachConsole(ATTACH_PARENT_PROCESS);
    }
}

#[cfg(not(windows))]
fn attach_console() {}
