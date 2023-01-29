#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]
mod window_style;
use parallel_disk_usage::args::Fraction;
use parallel_disk_usage::bytes_format::BytesFormat;
use parallel_disk_usage::reporter::{ErrorReport, ProgressAndErrorReporter};
use parallel_disk_usage::size_getters::GET_APPARENT_SIZE;
use parallel_disk_usage::visualizer::{BarAlignment, ColumnWidthDistribution, Direction};
use parallel_disk_usage::{self, reporter::ProgressReport, size::Size};
use regex::Regex;
use serde::Serialize;
use window_vibrancy::NSVisualEffectMaterial;
use std::borrow::Borrow;
use std::cell::RefCell;
use std::fmt::{Display, Error, Formatter, Write};
use std::fs;
use std::fs::metadata;
use std::io::Bytes;
use std::num::NonZeroUsize;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use std::{path::PathBuf, thread};
use sysinfo::{DiskExt, System, SystemExt};
use tauri::api::process::{Command as TauriCommand, CommandChild, CommandEvent};
use tauri::Manager;
use walkdir::WalkDir;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SquirrelDisk<'a> {
    name: &'a str,
    s_mount_point: String,
    total_space: u64,
    available_space: u64,
    is_removable: bool,
}

#[derive(Clone, serde::Serialize)]
struct Payload {
    items: u64,
    total: u64,
    errors: u64,
}

#[tauri::command]
fn show_in_folder(path: String) {
    #[cfg(target_os = "windows")]
    {
        let re = Regex::new(r"/").unwrap();
        let result = re.replace_all(&path, "\\");
        Command::new("explorer")
            .args(["/select,", format!("{}", result).as_str()]) // The comma after select is not a typo
            .spawn()
            .unwrap();
    }

    #[cfg(target_os = "linux")]
    {
        if path.contains(",") {
            // see https://gitlab.freedesktop.org/dbus/dbus/-/issues/76
            let new_path = match metadata(&path).unwrap().is_dir() {
                true => path,
                false => {
                    let mut path2 = PathBuf::from(path);
                    path2.pop();
                    path2.into_os_string().into_string().unwrap()
                }
            };
            Command::new("xdg-open").arg(&new_path).spawn().unwrap();
        } else {
            Command::new("dbus-send")
                .args([
                    "--session",
                    "--dest=org.freedesktop.FileManager1",
                    "--type=method_call",
                    "/org/freedesktop/FileManager1",
                    "org.freedesktop.FileManager1.ShowItems",
                    format!("array:string:\"file://{path}\"").as_str(),
                    "string:\"\"",
                ])
                .spawn()
                .unwrap();
        }
    }

    #[cfg(target_os = "macos")]
    {
        Command::new("open").args(["-R", &path]).spawn().unwrap();
    }
}
// Learn more about Tauri commands at https://tauri.app/v1/guides/features/command
#[tauri::command]
fn get_disks() -> String {
    let mut sys = System::new_all();
    sys.refresh_all();

    let mut vec: Vec<SquirrelDisk> = Vec::new();

    for disk in sys.disks() {
        vec.push(SquirrelDisk {
            name: disk.name().to_str().unwrap(),
            s_mount_point: disk.mount_point().display().to_string(),
            total_space: disk.total_space(),
            available_space: disk.available_space(),
            is_removable: disk.is_removable(),
        });
    }
    return serde_json::to_string(&vec).unwrap().into();
}
struct MyState(Mutex<Option<CommandChild>>);
#[tauri::command]
fn start_scanning(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, MyState>,
    path: String,
    ratio: String,
) -> Result<(), ()> {
    println!("Start Scanning {}", path);
    let ratio = ["--min-ratio=", ratio.as_str()].join("");
    let (mut rx, child) = TauriCommand::new_sidecar("pdu")
        .expect("failed to create `my-sidecar` binary command")
        .args(["--json-output", "--progress", ratio.as_str(), path.as_str()])
        .spawn()
        .expect("Failed to spawn sidecar");
    //

    *state.0.lock().unwrap() = Some(child);
    // let child = Arc::new(child);
    // let weak_child = Arc::clone(&child);
    // let id = app_handle.listen_global("scan_stop", |event| {
    //     weak_child.kill();
    // });
    // unlisten to the event using the `id` returned on the `listen_global` function
    // an `once_global` API is also exposed on the `App` struct

    let re = Regex::new(r"\(scanned ([0-9]*), total ([0-9]*)(?:, erred ([0-9]*))?\)").unwrap();
    // child.kill()
    tauri::async_runtime::spawn(async move {
        // read events such as stdout

        // let child_arc = Arc::new(child);

        // // On another thread, wait on the child process.
        // let child_arc_clone = child_arc.clone();

        while let Some(event) = rx.recv().await {
            match event {
                CommandEvent::Stdout(line) => {
                    // println!("Stdout:{}", &line);
                    app_handle.emit_all("scan_completed", line);
                }
                CommandEvent::Stderr(msg) => {
                    // println!("Stderr:{}", &msg);

                    let caps = re.captures(&msg);
                    if let Some(groups) = caps {
                        if groups.len() > 2 {
                            app_handle
                                .emit_all(
                                    "scan_status",
                                    Payload {
                                        items: groups
                                            .get(1)
                                            .map_or("0", |m| m.as_str())
                                            .trim_end()
                                            .parse::<u64>()
                                            .unwrap(),
                                        total: groups
                                            .get(2)
                                            .map_or("0", |m| m.as_str())
                                            .trim_end()
                                            .parse::<u64>()
                                            .unwrap(),
                                        errors: groups
                                            .get(3)
                                            .map_or("0", |m| m.as_str())
                                            .trim_end()
                                            .parse::<u64>()
                                            .unwrap(),
                                    },
                                )
                                .unwrap();
                        }
                    }
                }
                CommandEvent::Terminated(t) => {
                    println!("{t:?}");
                    // app_handle.unlisten(id);
                    // child.kill();
                }
                _ => unimplemented!(),
            };
            // if let CommandEvent::Stdout(line) = event {
            //     println!("StdErr: {}", line);
            // } else {
            //     println!("Terminated {}", event);
            // }
            // if let CommandEvent::Stderr(line) = event {
            //     println!("StdErr: {}", line);
            // }
            // if let CommandEvent::Terminated(line) = event {
            //     println!("Terminated");
            // }
        }
        Result::<(), ()>::Ok(())
    });

    Ok(())
    // thread::spawn(move || {
    //     let path = PathBuf::from(path);
    //     let mut vec: Vec<PathBuf> = Vec::new();
    //     vec.push(path);

    //     fn progress_and_error_reporter<Data>(
    //         app_handle: tauri::AppHandle,
    //     ) -> ProgressAndErrorReporter<Data, fn(ErrorReport)>
    //     where
    //         Data: Size + Into<u64> + Send + Sync,
    //         ProgressReport<Data>: Default + 'static,
    //         u64: Into<Data>,
    //     {
    //         let progress_reporter = move |report: ProgressReport<Data>| {
    //             let ProgressReport {
    //                 items,
    //                 total,
    //                 errors,
    //             } = report;
    //             let mut text = String::new();
    //             write!(
    //                 text,
    //                 "\r(scanned {items}, total {total}",
    //                 items = items,
    //                 total = total.into(),
    //             )
    //             .unwrap();
    //             if errors != 0 {
    //                 write!(text, ", erred {}", errors).unwrap();
    //             }
    //             write!(text, ")").unwrap();
    //             println!("{}", text);
    //             app_handle
    //                 .emit_all(
    //                     "scan_status",
    //                     Payload {
    //                         items: items,
    //                         total: total.into(),
    //                         errors: errors,
    //                     },
    //                 )
    //                 .unwrap();
    //         };

    //         struct TextReport<'a>(ErrorReport<'a>);

    //         impl<'a> Display for TextReport<'a> {
    //             fn fmt(&self, formatter: &mut Formatter<'_>) -> Result<(), Error> {
    //                 write!(
    //                     formatter,
    //                     "[error] {operation} {path:?}: {error}",
    //                     operation = self.0.operation.name(),
    //                     path = self.0.path,
    //                     error = self.0.error,
    //                 )
    //             }
    //         }

    //         let error_reporter: fn(ErrorReport) = |report| {
    //             let message = TextReport(report).to_string();
    //             println!("{}", message);
    //         };

    //         ProgressAndErrorReporter::new(
    //             progress_reporter,
    //             Duration::from_millis(100),
    //             error_reporter,
    //         )
    //     }
    //     // pub struct MyReporter {}
    //     // impl parallel_disk_usage::reporter::progress_and_error_reporter
    //     let pdu = parallel_disk_usage::app::Sub {
    //         json_output: true,
    //         direction: Direction::BottomUp,
    //         bar_alignment: BarAlignment::Right,
    //         get_data: GET_APPARENT_SIZE,
    //         files: vec,
    //         no_sort: true,
    //         min_ratio: 0.01.try_into().unwrap(),
    //         max_depth: 10.try_into().unwrap(),
    //         reporter: progress_and_error_reporter(app_handle),
    //         bytes_format: BytesFormat::MetricUnits,
    //         column_width_distribution: ColumnWidthDistribution::total(100),
    //     }
    //     .run();
    // });
}

#[tauri::command]
fn stop_scanning(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, MyState>,
    path: String,
) -> Result<(), ()> {
    state.0.lock().unwrap().take().unwrap().kill();
    Ok(())
}
fn main() {
    tauri::Builder::default()
        .manage(MyState(Default::default()))
        .setup(|app| {
            let window = app.get_window("main").unwrap();
            // window.open_devtools();
            #[cfg(target_os = "macos")]
            window_vibrancy::apply_vibrancy(&window, NSVisualEffectMaterial::HudWindow, None, None)
                .expect("Error aplying blurred bg");

            #[cfg(target_os = "windows")]
            window_vibrancy::apply_blur(&window, Some((18, 18, 18, 125)))
                .expect("Error aplying blurred bg");

            #[cfg(any(windows, target_os = "macos"))]
            window_style::set_window_styles(&window).unwrap();

            // app.listen_global("scan_stop", |event| {
            //     let s = app.state::<MyState>();
            //     s.0.lock().unwrap().take().unwrap().kill();
            // });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_disks,
            start_scanning,
            stop_scanning,
            show_in_folder
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
