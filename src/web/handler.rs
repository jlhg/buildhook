use std::fs::File;
use std::io::Write;
use std::process::Command;
use std::sync::Arc;

use axum::extract::{Extension, Json};
use axum::http::StatusCode;
use http::HeaderMap;
use reqwest::blocking::{multipart::Form, Client};
use serde_json::{json, Value};
use tempfile::tempdir;
use tracing::error;

use crate::crypto::verify_signature;
use crate::web::app::AppState;
use crate::web::object::*;

pub async fn post_webhooks_github(
    Extension(state): Extension<Arc<AppState>>,
    headers: HeaderMap,
    body: String,
    // Json(payload): Json<serde_json::Value>,
) -> (StatusCode, Json<Value>) {
    match headers.get("X-Hub-Signature-256") {
        Some(v) => {
            let signature = v.to_str().unwrap_or("");

            match verify_signature(
                state.config.github_webhook_secret.as_bytes(),
                body.as_bytes(),
                signature
                    .strip_prefix("sha256=")
                    .unwrap_or_default()
                    .as_bytes(),
            ) {
                Ok(matched) => {
                    if !matched {
                        return render_forbidden("invalid signature");
                    }
                }
                Err(e) => {
                    error!("{}", e);
                    return render_internal_server_error("can't verify signature");
                }
            }
        }
        None => return render_forbidden("missing signature"),
    }

    match headers.get("X-GitHub-Event") {
        Some(v) => {
            let event = v.to_str().unwrap_or("");
            match event {
                "ping" => return render_success(StatusCode::OK, "ping event ok"),
                "push" => {
                    let payload: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
                    if payload == Value::Null {
                        return render_bad_request("invalid payload");
                    }

                    let git_ref = payload["ref"].to_string();
                    if git_ref == "null" {
                        return render_bad_request("invalid [ref] value in the payload");
                    }

                    let branch = git_ref.strip_prefix("\"refs/heads/").unwrap_or("null");
                    if branch == "null" {
                        return render_bad_request("invalid stripped [ref] value in the payload");
                    }

                    if branch == state.config.github_watch_push_branch {
                        let repo = payload["repository"]["full_name"].to_string();
                        if repo == "null" {
                            return render_bad_request(
                                "invalid [repository.full_name] value in the payload",
                            );
                        }

                        let commit_id = payload["after"].to_string();
                        if commit_id == "null" {
                            return render_bad_request("invalid [after] value in the payload");
                        }

                        let head_commit_url = payload["head_commit"]["url"].to_string();
                        if head_commit_url == "null" {
                            return render_bad_request("invalid [head_commit.url] value in the payload");
                        }

                        let head_commit_committer_username = payload["head_commit"]["committer"]["username"].to_string();
                        if head_commit_committer_username == "null" {
                            return render_bad_request("invalid [head_commit.committer.username] value in the payload");
                        }

                        let output = Command::new(&state.config.build_entry_script_path)
                            .arg(&commit_id)
                            .output()
                            .expect("failed to execute the process");

                        let stdout_str = String::from_utf8(output.stdout)
                            .expect("failed to process stdout content");
                        let stderr_str = String::from_utf8(output.stderr)
                            .expect("failed to process stderr content");

                        let temp_dir = tempdir().expect("failed to create temporary directory");
                        let stdout_file_path = temp_dir.path().join("stdout.txt");
                        let stderr_file_path = temp_dir.path().join("stderr.txt");
                        let mut stdout_file =
                            File::create(&stdout_file_path).expect("failed to create stdout file");
                        let mut stderr_file =
                            File::create(&stderr_file_path).expect("failed to create stderr file");
                        write!(stdout_file, "{}", stdout_str).expect("failed to write stdout file");
                        write!(stderr_file, "{}", stderr_str).expect("failed to write stderr file");

                        let payload_json = if output.status.success() {
                            json!({
                                "embeds": [{
                                    "title": "Deployment Success",
                                    "url": head_commit_url,
                                    "color": 10731148, // #a3be8c
                                    "fields": [
                                        { "name": "Repository", "value": repo},
                                        { "name": "Branch", "value": branch, "inline": true},
                                        { "name": "Commit ID", "value": &commit_id[..7], "inline": true},
                                        { "name": "Committer", "value": head_commit_committer_username }
                                    ]
                                }]
                            })
                        } else {
                            json!({
                                "embeds": [{
                                    "title": "Deployment Failed",
                                    "url": head_commit_url,
                                    "color": 12542314, // "#bf616a"
                                    "fields": [
                                        { "name": "Repository", "value": repo},
                                        { "name": "Branch", "value": branch, "inline": true},
                                        { "name": "Commit ID", "value": &commit_id[..7], "inline": true},
                                        { "name": "Committer", "value": head_commit_committer_username }
                                    ]
                                }]
                            })
                        }
                        .to_string();

                        let form = Form::new()
                            .text("payload_json", payload_json)
                            .file("file1", &stdout_file_path)
                            .expect("failed to attach file1")
                            .file("file2", &stderr_file_path)
                            .expect("failed to attach file2");

                        let _resp = Client::new()
                            .post(&state.config.discord_webhook_url)
                            .multipart(form)
                            .send()
                            .expect("failed to send the request to Discord");

                        drop(stdout_file);
                        drop(stderr_file);
                        let _ = temp_dir.close();

                        return render_success(StatusCode::OK, "push event ok");
                    }

                    return render_success(StatusCode::OK, "unhandled branch");
                }
                _ => return render_success(StatusCode::OK, "unhandled event"),
            }
        }
        None => return render_success(StatusCode::OK, "no event"),
    }
}
