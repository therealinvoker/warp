use std::path::PathBuf;
use std::sync::Arc;

use warpui::r#async::executor::Background;

use super::display_working_directory;
use crate::ai::blocklist::agent_view::zero_state_block::current_working_directory_for_zero_state;
use crate::terminal::color::{self, Colors};
use crate::terminal::event_listener::ChannelEventListener;
use crate::terminal::model::ansi::{Handler, InitShellValue, PrecmdValue, SSHValue};
use crate::terminal::model::test_utils::block_size;
use crate::terminal::model::TerminalModel;

fn terminal_with_startup_path(startup_path: Option<&str>) -> TerminalModel {
    TerminalModel::new_for_test(
        block_size(),
        color::List::from(&Colors::default()),
        ChannelEventListener::new_for_test(),
        Arc::new(Background::default()),
        false,
        None,
        false,
        false,
        startup_path.map(PathBuf::from),
    )
}

fn prebootstrap_terminal_with_startup_path(startup_path: &str) -> TerminalModel {
    let mut terminal = terminal_with_startup_path(Some(startup_path));
    terminal.block_list_mut().reinit_shell();
    terminal
}

#[test]
fn display_working_directory_abbreviates_home_directory() {
    let display = display_working_directory(Some("/Users/alice"), Some("/Users/alice"));
    assert_eq!(display, Some("~".to_owned()));
}

#[test]
fn display_working_directory_abbreviates_subdirectory_under_home() {
    let display = display_working_directory(Some("/Users/alice/repo"), Some("/Users/alice"));
    assert_eq!(display, Some("~/repo".to_owned()));
}

#[test]
fn cwd_for_recent_conversations_prefers_active_block_pwd() {
    let mut terminal = prebootstrap_terminal_with_startup_path("/startup/path");
    terminal.precmd(PrecmdValue {
        pwd: Some("/active/path".to_owned()),
        session_id: Some(123),
        ..Default::default()
    });

    let cwd = current_working_directory_for_zero_state(&terminal);
    assert_eq!(cwd, Some("/active/path".to_owned()));
}

#[test]
fn cwd_for_recent_conversations_uses_startup_path_before_bootstrap_for_local_session() {
    let terminal = prebootstrap_terminal_with_startup_path("/startup/path");
    let cwd = current_working_directory_for_zero_state(&terminal);
    assert_eq!(cwd, Some("/startup/path".to_owned()));
}

#[test]
fn cwd_for_recent_conversations_does_not_use_startup_path_for_pending_ssh_bootstrap() {
    let mut terminal = prebootstrap_terminal_with_startup_path("/startup/path");

    terminal.ssh(SSHValue {
        session_id: Some(123),
        remote_session_id: Some(456),
        ..Default::default()
    });
    let cwd = current_working_directory_for_zero_state(&terminal);
    assert_eq!(cwd, None);
}

#[test]
fn cwd_for_recent_conversations_does_not_use_startup_path_for_pending_remote_session() {
    let mut terminal = prebootstrap_terminal_with_startup_path("/startup/path");
    terminal.init_shell(InitShellValue {
        session_id: 123.into(),
        shell: "zsh".to_owned(),
        hostname: "remote.example.com".to_owned(),
        ..Default::default()
    });

    let cwd = current_working_directory_for_zero_state(&terminal);
    assert_eq!(cwd, None);
}

#[test]
fn cwd_for_recent_conversations_does_not_use_startup_path_after_bootstrap() {
    let terminal = terminal_with_startup_path(Some("/startup/path"));
    let cwd = current_working_directory_for_zero_state(&terminal);
    assert_eq!(cwd, None);
}
