use emmylua_code_analysis::file_path_to_uri;
use log::{info, warn};
use lsp_types::{
    DidChangeWatchedFilesParams, DidChangeWatchedFilesRegistrationOptions, FileEvent,
    FileSystemWatcher, GlobPattern, Registration, RegistrationParams, Unregistration,
    UnregistrationParams, WatchKind,
};
use notify::{Config, RecommendedWatcher, RecursiveMode, Watcher};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

use crate::{
    context::{ClientProxy, ServerContextSnapshot},
    handlers::text_document::on_did_change_watched_files,
};
use emmylua_code_analysis::{WorkspaceFileMatcher, WorkspaceFolder};

const WATCH_FILES_REGISTRATION_ID: &str = "emmylua_watch_files";
const WATCHED_CONFIG_GLOBS: [&str; 4] = [
    "**/.editorconfig",
    "**/.luarc.json",
    "**/.emmyrc.json",
    "**/.emmyrc.lua",
];
const WATCHED_CONFIG_FILE_NAMES: [&str; 4] = [
    ".editorconfig",
    ".luarc.json",
    ".emmyrc.json",
    ".emmyrc.lua",
];

pub async fn register_files_watch(context: ServerContextSnapshot) {
    let supports_dynamic_watch = context
        .lsp_features()
        .supports_dynamic_watched_files_registration();
    let (watch_roots, workspace_folders, match_file_pattern) = {
        let workspace_manager = context.workspace_manager().lock().await;
        (
            reduce_watch_roots(workspace_manager.match_file_pattern.watch_roots()),
            workspace_manager.workspace_folders.clone(),
            workspace_manager.match_file_pattern.clone(),
        )
    };
    let watch_plan =
        build_watch_registration_plan(supports_dynamic_watch, &workspace_folders, watch_roots);

    if watch_plan.use_lsp_client {
        register_files_watch_use_lsp_client(
            context.client(),
            match_file_pattern.source_file_globs(),
        );
    } else if supports_dynamic_watch {
        unregister_files_watch_use_lsp_client(context.client());
    }

    if watch_plan.notify_roots.is_empty() {
        let mut workspace_manager = context.workspace_manager().lock().await;
        workspace_manager.watcher = None;
        return;
    }

    info!("use notify to watch files: {:?}", watch_plan.notify_roots);
    let registered = register_files_watch_use_fsnotify(
        context.clone(),
        watch_plan.notify_roots,
        match_file_pattern,
    )
    .await;
    if !registered {
        if watch_plan.use_lsp_client {
            warn!(
                "notify registration failed for external roots; continuing with client watched files"
            );
        } else {
            warn!("notify registration failed; no watched-file backend is available");
        }
    }
}

#[derive(Debug)]
struct WatchRegistrationPlan {
    use_lsp_client: bool,
    notify_roots: Vec<PathBuf>,
}

fn build_watch_registration_plan(
    supports_dynamic_watch: bool,
    workspace_folders: &[WorkspaceFolder],
    watch_roots: Vec<PathBuf>,
) -> WatchRegistrationPlan {
    if !supports_dynamic_watch {
        return WatchRegistrationPlan {
            use_lsp_client: false,
            notify_roots: watch_roots,
        };
    }

    WatchRegistrationPlan {
        use_lsp_client: !workspace_folders.is_empty(),
        notify_roots: watch_roots
            .into_iter()
            .filter(|root| !is_root_covered_by_workspace_folders(root, workspace_folders))
            .collect(),
    }
}

fn is_root_covered_by_workspace_folders(
    path: &Path,
    workspace_folders: &[WorkspaceFolder],
) -> bool {
    workspace_folders
        .iter()
        .any(|workspace| is_path_covered_by_workspace(path, &workspace.root))
}

fn register_files_watch_use_lsp_client(client: &ClientProxy, source_file_globs: &[String]) {
    unregister_files_watch_use_lsp_client(client);

    let options = DidChangeWatchedFilesRegistrationOptions {
        watchers: lsp_file_watchers(&build_watch_file_globs(source_file_globs)),
    };

    let registration = Registration {
        id: WATCH_FILES_REGISTRATION_ID.to_string(),
        method: "workspace/didChangeWatchedFiles".to_string(),
        register_options: Some(serde_json::to_value(options).unwrap()),
    };
    client.dynamic_register_capability(RegistrationParams {
        registrations: vec![registration],
    });
}

fn build_watch_file_globs(source_file_globs: &[String]) -> Vec<String> {
    let mut globs = source_file_globs.to_vec();
    globs.extend(WATCHED_CONFIG_GLOBS.into_iter().map(str::to_string));
    globs.sort();
    globs.dedup();
    globs
}

fn lsp_file_watchers(watch_file_globs: &[String]) -> Vec<FileSystemWatcher> {
    watch_file_globs
        .iter()
        .map(|glob_pattern| FileSystemWatcher {
            glob_pattern: GlobPattern::String(glob_pattern.clone()),
            kind: Some(WatchKind::Create | WatchKind::Change | WatchKind::Delete),
        })
        .collect()
}

fn unregister_files_watch_use_lsp_client(client: &ClientProxy) {
    client.dynamic_unregister_capability(UnregistrationParams {
        unregisterations: vec![Unregistration {
            id: WATCH_FILES_REGISTRATION_ID.to_string(),
            method: "workspace/didChangeWatchedFiles".to_string(),
        }],
    });
}

async fn register_files_watch_use_fsnotify(
    context: ServerContextSnapshot,
    watch_roots: Vec<PathBuf>,
    match_file_pattern: WorkspaceFileMatcher,
) -> bool {
    let (tx, rx) = unbounded_channel();
    let config = Config::default().with_poll_interval(Duration::from_secs(5));
    let mut watcher = match RecommendedWatcher::new(
        move |res| {
            if let Ok(event) = res {
                match tx.send(event) {
                    Ok(_) => {}
                    Err(e) => {
                        warn!("send notify event failed: {:?}", e);
                    }
                };
            };
        },
        config,
    ) {
        Ok(watcher) => watcher,
        Err(e) => {
            log::error!("create notify watcher failed: {:?}", e);
            return false;
        }
    };

    let mut watched_roots = Vec::new();
    for root in watch_roots {
        match watcher.watch(&root, RecursiveMode::Recursive) {
            Ok(()) => watched_roots.push(root),
            Err(e) => warn!("can not watch {:?}: {:?}", root, e),
        }
    }

    if watched_roots.is_empty() {
        warn!("can not watch any workspace roots with notify");
        return false;
    }

    let mut workspace_manager = context.workspace_manager().lock().await;
    workspace_manager.watcher = Some(watcher);
    drop(workspace_manager);

    tokio::spawn(forward_notify_events(
        rx,
        match_file_pattern,
        move |params| {
            let context = context.clone();
            async move {
                on_did_change_watched_files(context, params).await;
            }
        },
    ));

    info!("watch files use notify success: {:?}", watched_roots);
    true
}

async fn forward_notify_events<F, Fut>(
    mut rx: UnboundedReceiver<notify::Event>,
    match_file_pattern: WorkspaceFileMatcher,
    mut forward: F,
) where
    F: FnMut(DidChangeWatchedFilesParams) -> Fut,
    Fut: Future<Output = ()>,
{
    loop {
        match rx.recv().await {
            Some(event) => {
                let file_events = notify_event_to_file_events(&event, &match_file_pattern);
                if file_events.is_empty() {
                    continue;
                }
                forward(DidChangeWatchedFilesParams {
                    changes: file_events,
                })
                .await;
            }
            None => {
                warn!("watch files notify channel closed");
                break;
            }
        }
    }
}

fn notify_event_to_file_events(
    event: &notify::Event,
    match_file_pattern: &WorkspaceFileMatcher,
) -> Vec<FileEvent> {
    let typ = match event.kind {
        notify::event::EventKind::Create(_) => lsp_types::FileChangeType::CREATED,
        notify::event::EventKind::Modify(_) => lsp_types::FileChangeType::CHANGED,
        notify::event::EventKind::Remove(_) => lsp_types::FileChangeType::DELETED,
        _ => return Vec::new(),
    };
    event
        .paths
        .iter()
        .filter(|path| is_watched_path(match_file_pattern, path))
        .filter_map(|path| {
            // A rename reports Modify(Name) for the vanished old path; treat a missing file as deleted.
            let typ = if typ != lsp_types::FileChangeType::DELETED && !path.exists() {
                lsp_types::FileChangeType::DELETED
            } else {
                typ
            };
            Some(FileEvent::new(file_path_to_uri(path)?, typ))
        })
        .collect()
}

fn is_watched_path(match_file_pattern: &WorkspaceFileMatcher, path: &Path) -> bool {
    is_config_watch_path(path) || match_file_pattern.is_match(path)
}

fn is_config_watch_path(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| WATCHED_CONFIG_FILE_NAMES.contains(&name))
}

fn is_path_covered_by_workspace(path: &Path, workspace_root: &Path) -> bool {
    path.starts_with(workspace_root)
}

fn reduce_watch_roots<I>(roots: I) -> Vec<PathBuf>
where
    I: IntoIterator<Item = PathBuf>,
{
    let mut roots = roots.into_iter().collect::<Vec<_>>();
    roots.sort();

    let mut reduced_roots: Vec<PathBuf> = Vec::new();
    for root in roots {
        if reduced_roots
            .iter()
            .any(|existing| is_path_covered_by_workspace(&root, existing))
        {
            continue;
        }

        reduced_roots.retain(|existing| !is_path_covered_by_workspace(existing, &root));
        reduced_roots.push(root);
    }

    reduced_roots
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[test]
    fn rename_away_from_a_watched_path_reports_deleted() {
        let root = std::env::temp_dir().join(format!("emmylua_ls_watch_{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let old_path = root.join("old.lua");
        let new_path = root.join("new.lua");
        std::fs::write(&new_path, "").unwrap();
        let matcher = WorkspaceFileMatcher::new(
            &[WorkspaceFolder::new(root.clone(), false)],
            &emmylua_code_analysis::Emmyrc::default(),
        );

        let from = notify::Event::new(notify::EventKind::Modify(notify::event::ModifyKind::Name(
            notify::event::RenameMode::From,
        )))
        .add_path(old_path.clone());
        let to = notify::Event::new(notify::EventKind::Modify(notify::event::ModifyKind::Name(
            notify::event::RenameMode::To,
        )))
        .add_path(new_path.clone());
        let from_events = notify_event_to_file_events(&from, &matcher);
        let to_events = notify_event_to_file_events(&to, &matcher);
        std::fs::remove_dir_all(&root).unwrap();

        assert_eq!(from_events.len(), 1);
        assert_eq!(from_events[0].typ, lsp_types::FileChangeType::DELETED);
        assert_eq!(to_events.len(), 1);
        assert_eq!(to_events[0].typ, lsp_types::FileChangeType::CHANGED);
    }

    #[test]
    fn idle_forwarder_does_not_block_other_tasks() {
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .build()
                .unwrap();
            let ran = runtime.block_on(async {
                let (_event_tx, event_rx) = unbounded_channel();
                tokio::spawn(forward_notify_events(
                    event_rx,
                    WorkspaceFileMatcher::default(),
                    |_| async {},
                ));
                let ran = Arc::new(AtomicBool::new(false));
                let flag = ran.clone();
                tokio::spawn(async move { flag.store(true, Ordering::SeqCst) });
                tokio::task::yield_now().await;
                tokio::task::yield_now().await;
                ran.load(Ordering::SeqCst)
            });
            let _ = done_tx.send(ran);
        });

        let ran = done_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("the idle notify forwarder blocked the runtime thread");
        assert!(ran);
    }
}
