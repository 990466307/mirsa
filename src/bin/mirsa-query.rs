#![feature(rustc_private)]

use mirsa::{Error, Mirsa, SourceBindings, query::Report};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, io, path::PathBuf, process::ExitCode};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    #[serde(default)]
    file: Option<PathBuf>,
    dsl: String,
    bindings: BTreeMap<String, String>,
    #[serde(default)]
    rustc_args: Vec<String>,
    #[serde(default)]
    source: Option<String>,
}

#[derive(Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum Response {
    Ok { report: Report },
    Error { kind: &'static str, message: String },
}

impl Response {
    fn error(kind: &'static str, message: impl ToString) -> Self {
        Self::Error {
            kind,
            message: message.to_string(),
        }
    }
}

fn query(request: Request) -> Response {
    let file = match request.file.as_ref() {
        Some(path) => match path.canonicalize() {
            Ok(file) if file.is_file() => Some(file),
            Ok(_) => return Response::error("input", "file 必须是 Rust 源文件"),
            Err(error) => return Response::error("input", error),
        },
        None => None,
    };
    let bindings = request
        .bindings
        .into_iter()
        .fold(SourceBindings::new(), |bindings, (name, source)| {
            bindings.variable(name, source)
        });
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
    match mirsa
        .rustc_args(request.rustc_args)
        .query(&request.dsl, &bindings)
    {
        Ok(report) => Response::Ok { report },
        Err(error) => {
            let kind = match &error {
                Error::Compilation => "compilation",
                Error::Binding(_) => "binding",
                Error::Query(_) => "query",
            };
            Response::error(kind, error)
        }
    }
}

fn main() -> ExitCode {
    let response = match serde_json::from_reader(io::stdin().lock()) {
        Ok(request) => query(request),
        Err(error) => Response::error("input", error),
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
