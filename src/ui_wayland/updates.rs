use crate::wayland_update::{self, Release};
use gtk::gio;
use gtk::prelude::*;
use std::cell::RefCell;
use std::sync::mpsc;
use std::thread;

enum State {
    Idle,
    Checking,
    Available(Release),
}

pub(super) struct UpdateControl {
    pub button: gtk::Button,
    state: RefCell<State>,
    sender: mpsc::Sender<Result<Option<Release>, String>>,
    results: mpsc::Receiver<Result<Option<Release>, String>>,
}

impl UpdateControl {
    pub fn new() -> Self {
        let (sender, results) = mpsc::channel();
        let button = gtk::Button::with_label("检查更新");
        Self { button, state: RefCell::new(State::Idle), sender, results }
    }

    pub fn activate(&self) {
        if let State::Available(release) = &*self.state.borrow() {
            if gio::AppInfo::launch_default_for_uri(&release.download_url, None::<&gio::AppLaunchContext>).is_err() {
                self.button.set_tooltip_text(Some(&format!("无法打开浏览器。下载地址为 {}", release.download_url)));
            }
            return;
        }
        self.check();
    }

    pub fn check(&self) {
        if matches!(*self.state.borrow(), State::Checking) {
            return;
        }
        *self.state.borrow_mut() = State::Checking;
        self.button.set_label("检查更新中…");
        self.button.set_tooltip_text(None);
        self.button.set_sensitive(false);
        let sender = self.sender.clone();
        thread::spawn(move || { let _ = sender.send(wayland_update::check()); });
    }

    pub fn poll(&self) {
        let Ok(result) = self.results.try_recv() else { return };
        self.button.set_sensitive(true);
        match result {
            Ok(Some(release)) => {
                self.button.set_label(&format!("下载 Wayland v{}", release.version));
                self.button.set_tooltip_text(Some("打开这个版本的 Linux x86_64 下载链接"));
                *self.state.borrow_mut() = State::Available(release);
            }
            Ok(None) => {
                self.button.set_label("已是最新版");
                self.button.set_tooltip_text(Some("再次检查 Linux Wayland 版本"));
                *self.state.borrow_mut() = State::Idle;
            }
            Err(error) => {
                self.button.set_label("更新检查失败 · 重试");
                self.button.set_tooltip_text(Some(&error));
                *self.state.borrow_mut() = State::Idle;
            }
        }
    }
}
