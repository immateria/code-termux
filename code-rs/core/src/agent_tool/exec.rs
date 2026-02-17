use super::*;

use crate::agent_defaults::{agent_model_spec, default_params_for};
use crate::config_types::AgentConfig;
use crate::spawn::spawn_tokio_command_with_retry;
use shlex::split as shlex_split;
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration as StdDuration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::runtime::Builder as TokioRuntimeBuilder;
use tokio::time::Duration as TokioDuration;
use uuid::Uuid;

mod cloud;
mod command;
mod process_output;
mod runner;
mod runtime_paths;
mod smoke;

pub(crate) use command::maybe_set_gemini_config_dir;
pub(crate) use runtime_paths::current_code_binary_path;
pub(crate) use runtime_paths::resolve_program_path;
pub(crate) use runtime_paths::should_use_current_exe_for_agent;
pub(crate) use runner::ExecuteModelRequest;
pub(crate) use runner::execute_agent;
pub(crate) use runner::execute_model_with_permissions;
#[cfg(test)]
pub(crate) use runner::prefer_json_result;
pub use command::split_command_and_args;
pub use runtime_paths::external_agent_command_exists;
pub use smoke::smoke_test_agent_blocking;
