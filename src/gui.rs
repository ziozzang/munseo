//! egui 드래그앤드롭 창. macOS·Windows 공통.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};

use eframe::egui;

use crate::batch::{self, Outcome};
use crate::convert::OutputFormat;
use crate::fonts;

pub fn run() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("HWP2PDF")
            .with_inner_size([640.0, 560.0])
            .with_min_inner_size([480.0, 400.0])
            .with_drag_and_drop(true)
            .with_icon(
                eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon/icon-256.png"))
                    .unwrap_or_default(),
            ),
        ..Default::default()
    };
    eframe::run_native("HWP2PDF", options, Box::new(|cc| Ok(Box::new(App::new(cc)))))
}

struct Job {
    id: usize,
    path: PathBuf,
    format: OutputFormat,
    delete_original: bool,
}

enum Msg {
    /// 드롭한 경로를 훑은 결과
    Found { files: Vec<PathBuf>, format: OutputFormat, delete_original: bool },
    Started(usize),
    /// (작업 id, 0~1, 지금 하는 일)
    Progress(usize, f32, String),
    Finished(usize, Outcome),
}

enum Status {
    Queued,
    Running { progress: f32, stage: String, started: std::time::Instant },
    Done(Outcome),
}

struct Row {
    input: PathBuf,
    status: Status,
    /// 끝난 파일의 걸린 시간
    elapsed: Option<std::time::Duration>,
}

struct App {
    format: OutputFormat,
    delete_original: bool,
    rows: Vec<Row>,
    notice: Option<String>,
    jobs: Sender<Job>,
    msg_tx: Sender<Msg>,
    msg_rx: Receiver<Msg>,
}

#[derive(Default)]
struct Counts {
    total: usize,
    queued: usize,
    running: usize,
    ok: usize,
    failed: usize,
    /// 진행 중인 파일들의 진행률 합
    partial: f32,
}

const KEY_FORMAT: &str = "format";
const KEY_DELETE: &str = "delete_original";

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        install_korean_font(&cc.egui_ctx);

        let (msg_tx, msg_rx) = mpsc::channel::<Msg>();
        let (jobs, job_rx) = mpsc::channel::<Job>();
        let job_rx = Arc::new(Mutex::new(job_rx));

        // 문서 한 개 PDF 변환에 수백 MB 를 쓸 수 있어 동시 작업 수를 제한한다.
        let workers = std::thread::available_parallelism()
            .map(|n| (n.get() / 2).clamp(1, 4))
            .unwrap_or(2);
        for _ in 0..workers {
            let job_rx = Arc::clone(&job_rx);
            let msg_tx = msg_tx.clone();
            let ctx = cc.egui_ctx.clone();
            std::thread::spawn(move || loop {
                let job = match job_rx.lock().unwrap().recv() {
                    Ok(job) => job,
                    Err(_) => return,
                };
                let _ = msg_tx.send(Msg::Started(job.id));
                ctx.request_repaint();
                // 진행 보고는 1% 이상 바뀌었거나 단계가 바뀔 때만 보낸다 (화면 갱신 폭주 방지).
                let last = std::cell::RefCell::new((-1.0f32, String::new()));
                let report = |p: f32, stage: String| {
                    let mut l = last.borrow_mut();
                    let stage_kind: String = stage.chars().filter(|c| !c.is_ascii_digit()).collect();
                    let last_kind: String = l.1.chars().filter(|c| !c.is_ascii_digit()).collect();
                    if p - l.0 >= 0.01 || stage_kind != last_kind || p >= 1.0 {
                        *l = (p, stage.clone());
                        let _ = msg_tx.send(Msg::Progress(job.id, p, stage));
                        ctx.request_repaint();
                    }
                };
                let outcome = batch::process(&job.path, job.format, job.delete_original, &report);
                let _ = msg_tx.send(Msg::Finished(job.id, outcome));
                ctx.request_repaint();
            });
        }

        let (format, delete_original) = cc
            .storage
            .map(|s| {
                (
                    eframe::get_value(s, KEY_FORMAT).unwrap_or(OutputFormat::PdfA),
                    eframe::get_value(s, KEY_DELETE).unwrap_or(false),
                )
            })
            .unwrap_or((OutputFormat::PdfA, false));

        Self {
            format,
            delete_original,
            rows: Vec::new(),
            notice: None,
            jobs,
            msg_tx,
            msg_rx,
        }
    }

    /// 폴더 탐색은 오래 걸릴 수 있으니 별도 스레드에서 한다.
    fn add_paths(&mut self, ctx: &egui::Context, paths: Vec<PathBuf>) {
        if paths.is_empty() {
            return;
        }
        let tx = self.msg_tx.clone();
        let ctx = ctx.clone();
        let (format, delete_original) = (self.format, self.delete_original);
        self.notice = Some("파일을 찾는 중…".into());
        std::thread::spawn(move || {
            let files = batch::collect(&paths);
            let _ = tx.send(Msg::Found { files, format, delete_original });
            ctx.request_repaint();
        });
    }

    fn drain_messages(&mut self) {
        while let Ok(msg) = self.msg_rx.try_recv() {
            match msg {
                Msg::Found { files, format, delete_original } => {
                    let mut added = 0;
                    for path in files {
                        let busy = self.rows.iter().any(|r| {
                            r.input == path && matches!(r.status, Status::Queued | Status::Running { .. })
                        });
                        if busy {
                            continue;
                        }
                        let id = self.rows.len();
                        self.rows.push(Row { input: path.clone(), status: Status::Queued, elapsed: None });
                        let _ = self.jobs.send(Job { id, path, format, delete_original });
                        added += 1;
                    }
                    self.notice = Some(if added == 0 {
                        "HWP/HWPX 파일을 찾지 못했습니다.".into()
                    } else {
                        format!("{added}개 파일을 {} 로 변환합니다.", format.label())
                    });
                }
                Msg::Started(id) => {
                    if let Some(row) = self.rows.get_mut(id) {
                        row.status = Status::Running {
                            progress: 0.0,
                            stage: "시작".into(),
                            started: std::time::Instant::now(),
                        };
                    }
                }
                Msg::Progress(id, p, text) => {
                    if let Some(Row { status: Status::Running { progress, stage, .. }, .. }) = self.rows.get_mut(id) {
                        *progress = p;
                        *stage = text;
                    }
                }
                Msg::Finished(id, outcome) => {
                    if let Some(row) = self.rows.get_mut(id) {
                        if let Status::Running { started, .. } = &row.status {
                            row.elapsed = Some(started.elapsed());
                        }
                        row.status = Status::Done(outcome);
                    }
                }
            }
        }
    }

    fn counts(&self) -> Counts {
        let mut c = Counts { total: self.rows.len(), ..Default::default() };
        for r in &self.rows {
            match &r.status {
                Status::Queued => c.queued += 1,
                Status::Running { progress, .. } => {
                    c.running += 1;
                    c.partial += *progress;
                }
                Status::Done(Outcome::Ok { .. }) => c.ok += 1,
                Status::Done(Outcome::Failed(_)) => c.failed += 1,
            }
        }
        c
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = &ui.ctx().clone();
        self.drain_messages();

        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw.dropped_files.iter().filter_map(|f| f.path.clone()).collect()
        });
        self.add_paths(ctx, dropped);
        let hovering = ctx.input(|i| !i.raw.hovered_files.is_empty());

        egui::Panel::top("options").show(ui, |ui| {
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.label("출력 포맷");
                egui::ComboBox::from_id_salt("format")
                    .selected_text(self.format.label())
                    .width(240.0)
                    .show_ui(ui, |ui| {
                        for f in OutputFormat::ALL {
                            ui.selectable_value(&mut self.format, f, f.label());
                        }
                    });
            });
            ui.add_space(4.0);
            ui.checkbox(&mut self.delete_original, "변환에 성공하면 원본 HWP 삭제 (휴지통으로 이동)");
            ui.add_space(8.0);
        });

        egui::CentralPanel::default().show(ui, |ui| {
            // 드롭 영역
            let accent = ui.visuals().selection.bg_fill;
            let stroke = if hovering {
                egui::Stroke::new(3.0, accent)
            } else {
                egui::Stroke::new(1.5, ui.visuals().widgets.noninteractive.bg_stroke.color)
            };
            egui::Frame::group(ui.style())
                .stroke(stroke)
                .corner_radius(10.0)
                .inner_margin(egui::Margin::same(20))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.vertical_centered(|ui| {
                        ui.heading(if hovering { "여기에 놓으세요" } else { "HWP 파일이나 폴더를 끌어다 놓으세요" });
                        ui.add_space(4.0);
                        ui.label("폴더를 놓으면 하위 폴더의 .hwp / .hwpx 까지 모두 변환합니다.");
                        ui.label("결과 파일은 원본과 같은 폴더에 저장됩니다.");
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            let w = 200.0;
                            ui.add_space((ui.available_width() - w).max(0.0) / 2.0);
                            if ui.button("파일 선택…").clicked() {
                                if let Some(files) = rfd::FileDialog::new()
                                    .add_filter("한글 문서", &["hwp", "hwpx", "HWP", "HWPX"])
                                    .pick_files()
                                {
                                    self.add_paths(ctx, files);
                                }
                            }
                            if ui.button("폴더 선택…").clicked() {
                                if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                                    self.add_paths(ctx, vec![dir]);
                                }
                            }
                        });
                    });
                });

            ui.add_space(8.0);
            let c = self.counts();
            if c.running > 0 {
                // 경과 시간이 멈춰 보이지 않게 변환 중에는 주기적으로 다시 그린다.
                ctx.request_repaint_after(std::time::Duration::from_millis(250));
            }
            let (total, done, failed) = (c.total, c.ok, c.failed);
            if total > 0 {
                let finished = done + failed;
                // 전체 진행률: 끝난 파일 + 진행 중인 파일의 부분 진행
                let overall = (finished as f32 + c.partial) / total as f32;
                ui.add(
                    egui::ProgressBar::new(overall)
                        .text(format!("전체 {:.0}%  ·  {finished} / {total} 파일", overall * 100.0)),
                );
                let status = if c.running + c.queued == 0 {
                    format!("모두 끝났습니다 — 성공 {done}, 실패 {failed}")
                } else {
                    format!("변환 중 {} · 대기 {} · 성공 {done} · 실패 {failed}", c.running, c.queued)
                };
                ui.label(egui::RichText::new(status).strong());
            }
            ui.horizontal(|ui| {
                if let Some(n) = &self.notice {
                    ui.label(n);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add_enabled(total > 0 && done + failed == total, egui::Button::new("목록 지우기"))
                        .clicked()
                    {
                        clear_finished(&mut self.rows);
                    }
                });
            });
            ui.separator();

            egui::ScrollArea::vertical().auto_shrink([false, false]).stick_to_bottom(true).show(ui, |ui| {
                for row in &self.rows {
                    row_ui(ui, row);
                }
            });
        });
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, KEY_FORMAT, &self.format);
        eframe::set_value(storage, KEY_DELETE, &self.delete_original);
    }
}

/// 작업 id 가 곧 행 번호이므로, 모든 작업이 끝났을 때만 목록을 비운다.
fn clear_finished(rows: &mut Vec<Row>) {
    if rows.iter().all(|r| matches!(r.status, Status::Done(_))) {
        rows.clear();
    }
}

fn row_ui(ui: &mut egui::Ui, row: &Row) {
    let name = row.input.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    ui.horizontal(|ui| {
        match &row.status {
            Status::Queued => {
                ui.label("⏳");
                ui.label(&name).on_hover_text(row.input.display().to_string());
                ui.weak("대기");
            }
            Status::Running { progress, stage, started } => {
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(&name).on_hover_text(row.input.display().to_string());
                        ui.weak(format!("{:.0}초", started.elapsed().as_secs_f32()));
                    });
                    ui.add(
                        egui::ProgressBar::new(*progress)
                            .desired_width(ui.available_width().min(480.0))
                            .text(format!("{:.0}% · {stage}", progress * 100.0)),
                    );
                });
            }
            Status::Done(Outcome::Ok { output, pages, original_deleted, warnings }) => {
                if warnings.is_empty() {
                    ui.colored_label(egui::Color32::from_rgb(40, 160, 70), "✔");
                } else {
                    ui.colored_label(egui::Color32::from_rgb(220, 160, 30), "⚠")
                        .on_hover_text(warnings.join("\n"));
                }
                ui.label(&name).on_hover_text(row.input.display().to_string());
                let out = output.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                match row.elapsed {
                    Some(t) => ui.weak(format!("{out} · {pages}쪽 · {:.1}초", t.as_secs_f32())),
                    None => ui.weak(format!("{out} · {pages}쪽")),
                };
                if *original_deleted {
                    ui.weak("· 원본 삭제됨");
                }
                for w in warnings {
                    ui.colored_label(egui::Color32::from_rgb(220, 160, 30), w);
                }
                if ui.small_button("보기").on_hover_text("Finder/탐색기에서 보기").clicked() {
                    reveal(output);
                }
            }
            Status::Done(Outcome::Failed(err)) => {
                ui.colored_label(egui::Color32::from_rgb(210, 60, 60), "✖");
                ui.label(&name).on_hover_text(row.input.display().to_string());
                ui.colored_label(egui::Color32::from_rgb(210, 60, 60), err);
            }
        }
    });
}

fn reveal(path: &Path) {
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg("-R").arg(path).spawn();
    #[cfg(target_os = "windows")]
    let _ = std::process::Command::new("explorer")
        .arg(format!("/select,{}", path.display()))
        .spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let _ = std::process::Command::new("xdg-open")
        .arg(path.parent().unwrap_or(path))
        .spawn();
}

/// egui 기본 폰트에는 한글이 없으므로 내장 Noto Sans KR 을 폴백으로 붙인다.
fn install_korean_font(ctx: &egui::Context) {
    let mut defs = egui::FontDefinitions::default();
    defs.font_data.insert(
        "noto_sans_kr".into(),
        Arc::new(egui::FontData::from_static(fonts::NOTO_SANS_KR)),
    );
    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        defs.families.entry(family).or_default().push("noto_sans_kr".into());
    }
    ctx.set_fonts(defs);
}
