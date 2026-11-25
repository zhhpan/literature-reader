use crate::update::{self, Release, VerifiedInstaller};
use std::borrow::Cow;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread;

enum State {
    Idle,
    Checking { manual: bool },
    Available(Release),
    Downloading { release: Release, percent: u8 },
    Ready { version: String, installer: VerifiedInstaller },
    Launching,
}

enum Completed {
    Checked(Option<Release>),
    Downloaded(VerifiedInstaller),
}

enum Event {
    Progress(u8),
    Finished(Result<Completed, String>),
}

pub(super) enum Intent {
    Check,
    Download(String),
    Install(String),
}

pub(super) struct Button {
    pub label: Cow<'static, str>,
    pub enabled: bool,
    pub primary: bool,
}

pub(super) struct UpdateControl {
    state: State,
    events: Option<Receiver<Event>>,
}

impl UpdateControl {
    pub(super) fn new() -> Self {
        Self { state: State::Idle, events: None }
    }

    pub(super) fn button(&self) -> Button {
        let (label, enabled, primary) = match &self.state {
            State::Idle => ("检查更新".into(), true, false),
            State::Checking { .. } => ("检查更新中…".into(), false, false),
            State::Available(..) => ("下载更新".into(), true, true),
            State::Downloading { percent: 100, .. } => ("正在校验…".into(), false, false),
            State::Downloading { percent, .. } => (format!("下载中 {percent}%").into(), false, false),
            State::Ready { .. } => ("安装更新".into(), true, true),
            State::Launching => ("打开安装程序…".into(), false, false),
        };
        Button { label, enabled, primary }
    }

    pub(super) fn intent(&self) -> Option<Intent> {
        match &self.state {
            State::Idle => Some(Intent::Check),
            State::Available(release) => Some(Intent::Download(release.version.clone())),
            State::Ready { version, .. } => Some(Intent::Install(version.clone())),
            _ => None,
        }
    }

    pub(super) fn check(&mut self, manual: bool) -> Option<String> {
        if !matches!(self.state, State::Idle) {
            return None;
        }
        self.start(State::Checking { manual }, |_| update::check().map(Completed::Checked))
    }

    pub(super) fn download(&mut self) -> Option<String> {
        let State::Available(release) = &self.state else { return None };
        let release = release.clone();
        self.start(State::Downloading { release: release.clone(), percent: 0 }, move |send| {
            let mut last = 0;
            update::download(&release, |written, total| {
                let percent = (u128::from(written) * 100 / u128::from(total.max(1))).min(100) as u8;
                if percent != last {
                    last = percent;
                    let _ = send.send(Event::Progress(percent));
                }
            }).map(Completed::Downloaded)
        })
    }

    fn start(
        &mut self, state: State,
        job: impl FnOnce(&Sender<Event>) -> Result<Completed, String> + Send + 'static,
    ) -> Option<String> {
        let (send, events) = mpsc::channel();
        let worker = thread::Builder::new().spawn(move || {
            let result = job(&send);
            let _ = send.send(Event::Finished(result));
        });
        if worker.is_err() {
            return (!matches!(state, State::Checking { manual: false }))
                .then(|| "无法启动更新任务，请重试。".into());
        }
        self.state = state;
        self.events = Some(events);
        None
    }

    pub(super) fn poll(&mut self) -> (bool, Option<String>) {
        let mut changed = false;
        while let Some(events) = &self.events {
            let result = match events.try_recv() {
                Ok(Event::Progress(percent)) => {
                    if let State::Downloading { percent: shown, .. } = &mut self.state {
                        changed |= *shown != percent;
                        *shown = percent;
                    }
                    continue;
                }
                Ok(Event::Finished(result)) => result,
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => Err("更新任务已中断，请重试。".into()),
            };
            self.events = None;
            return (true, self.finish(result));
        }
        (changed, None)
    }

    fn finish(&mut self, result: Result<Completed, String>) -> Option<String> {
        match (std::mem::replace(&mut self.state, State::Idle), result) {
            (State::Checking { .. }, Ok(Completed::Checked(Some(release)))) => {
                self.state = State::Available(release);
            }
            (State::Checking { manual: true }, Ok(Completed::Checked(None))) => {
                return Some(format!("当前版本 {} 已是最新版本。", update::CURRENT_VERSION));
            }
            (State::Downloading { release, .. }, Ok(Completed::Downloaded(installer))) => {
                self.put_installer_back(release.version, installer);
            }
            (State::Downloading { release, .. }, Err(error)) => {
                self.state = State::Available(release);
                return Some(error);
            }
            (State::Checking { manual: true }, Err(error)) => return Some(error),
            _ => {}
        }
        None
    }

    pub(super) fn take_installer(&mut self) -> Option<(String, VerifiedInstaller)> {
        match std::mem::replace(&mut self.state, State::Launching) {
            State::Ready { version, installer } => Some((version, installer)),
            previous => {
                self.state = previous;
                None
            }
        }
    }

    pub(super) fn put_installer_back(&mut self, version: String, installer: VerifiedInstaller) {
        self.state = State::Ready { version, installer };
    }
}
