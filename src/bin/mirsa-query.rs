#![feature(rustc_private)]

use mirsa::{Error, Mirsa};
use serde::{Deserialize, Serialize};
use std::{io, path::PathBuf, process::ExitCode};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    #[serde(default)]
    file: Option<PathBuf>,
    #[serde(default)]
    rustc_args: Vec<String>,
    #[serde(default)]
    source: Option<String>,
}

#[derive(Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum Response {
    Ok {
        reports: Vec<mirsa::AnnotationReport>,
    },
    Error {
        kind: &'static str,
        message: String,
        #[serde(skip_serializing_if = "Vec::is_empty")]
        diagnostics: Vec<mirsa::query::Diagnostic>,
    },
}

impl Response {
    fn error(kind: &'static str, message: impl ToString) -> Self {
        Self::Error {
            kind,
            message: message.to_string(),
            diagnostics: Vec::new(),
        }
    }
}

fn query(request: Request) -> Response {
    if request.file.is_some() && request.source.is_some() {
        return Response::error("input", "file 和 source 必须且只能提供一个");
    }
    let file = match request.file.as_ref() {
        Some(path) => match path.canonicalize() {
            Ok(file) if file.is_file() => Some(file),
            Ok(_) => return Response::error("input", "file 必须是 Rust 源文件"),
            Err(error) => return Response::error("input", error),
        },
        None => None,
    };
    let mirsa = match request.source {
        Some(source) => Mirsa::from_source(source),
        None => match file {
            Some(file) => Mirsa::from_file(file),
            None => {
                return Response::error(
                    "input",
                    "需要 file（磁盘上的 .rs 路径）或 source（内存里的源码字符串）之一",
                );
            }
        },
    };
    let mirsa = mirsa.rustc_args(request.rustc_args);
    let result = mirsa
        .analyze_annotations()
        .map(|reports| Response::Ok { reports });
    result.unwrap_or_else(|error| {
        let kind = match &error {
            Error::Compilation => "compilation",
            Error::Binding(_) => "binding",
            Error::Query(_) => "query",
        };
        let diagnostics = match &error {
            Error::Query(errors) => errors.clone(),
            _ => Vec::new(),
        };
        Response::Error {
            kind,
            message: error.to_string(),
            diagnostics,
        }
    })
}

fn main() -> ExitCode {
    let args = std::env::args_os().skip(1).collect::<Vec<_>>();
    let response = match args.as_slice() {
        [] => match serde_json::from_reader(io::stdin().lock()) {
            Ok(request) => query(request),
            Err(error) => Response::error("input", error),
        },
        [file] => query(Request {
            file: Some(file.into()),
            rustc_args: vec![],
            source: None,
        }),
        _ => Response::error(
            "input",
            "用法：mirsa-query <file.rs>；或不带参数从 stdin 读取 JSON",
        ),
    };
    let success = matches!(&response, Response::Ok { .. });
    if let Err(error) = serde_json::to_writer(io::stdout().lock(), &response) {
        eprintln!("无法写入查询结果：{error}");
        return ExitCode::FAILURE;
    }
    if success {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
