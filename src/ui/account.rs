use super::*;

pub(super) struct AccountPanel {
    pub(super) bounds: RECT,
    pub(super) first_hit: usize,
}

impl View {
    pub(super) fn account_panel_at(&self, point: POINT) -> bool {
        self.account_open && self.account_panel.as_ref().is_some_and(|panel| contains(panel.bounds, point))
    }

    pub(super) fn dismiss_account(&mut self) -> bool {
        std::mem::take(&mut self.account_open)
    }

    pub(super) fn open_account(&mut self) {
        self.account_open = true;
        self.reader.refresh_account();
    }

    pub(super) fn account_balance_label(&self) -> Option<String> {
        let credits = self.reader.account.as_ref()?.remaining_credits?;
        crate::translation::format_credit_balance(credits)
    }

    pub(super) fn paint_account(&mut self, painter: &Painter, bounds: RECT) {
        let border = if self.account_open || self.hover == Some(Action::Account) { ACCENT } else { LINE };
        self.paint_avatar(painter, bounds, border);
        let badge = rc(bounds.right - self.s(10), bounds.bottom - self.s(10), bounds.right, bounds.bottom);
        painter.round(badge, self.s(10), ACCENT, WHITE);
        self.hits.push(Hit { rect: bounds, action: Action::Account });
    }

    pub(super) fn paint_account_panel(&mut self, painter: &Painter, width: i32) {
        let dpi = self.dpi;
        let s = |px| scale(px, dpi);
        let right = width - s(22);
        let bounds = rc(right - s(342), s(56), right, s(329));
        let left = bounds.left + s(16);
        let inner_right = right - s(16);
        let top = bounds.top;
        let shell = rc(bounds.left + s(2), top + s(5), bounds.right + s(2), bounds.bottom + s(5));
        painter.round(shell, s(18), color(227, 234, 232), color(227, 234, 232));
        painter.round(bounds, s(18), WHITE, LINE);
        self.account_panel = Some(AccountPanel { bounds, first_hit: self.hits.len() });

        self.paint_avatar(painter, rc(left, top + s(16), left + s(48), top + s(64)), LINE);
        let account = self.reader.account.as_ref();
        let name = account.and_then(|account| account.display_name.as_deref()).unwrap_or("CodeBuddy");
        let signing_in = self.reader.signing_in();
        let header_status = if signing_in { "等待 CodeBuddy 授权" } else { "CodeBuddy 账户" };
        painter.line_text(name, TITLE, TEXT, rc(left + s(60), top + s(15), inner_right, top + s(44)), DT_LEFT);
        painter.line_text(header_status, SMALL, MUTED,
            rc(left + s(60), top + s(45), inner_right, top + s(65)), DT_LEFT);

        let summary = rc(left, top + s(80), inner_right, top + s(163));
        painter.round(summary, s(12), TINT, TINT);
        let plan_left = left + s(209);
        painter.fill(rc(plan_left - s(15), top + s(96), plan_left - s(14), top + s(147)), LINE);
        painter.line_text("剩余积分", SMALL, MUTED, rc(left + s(13), top + s(87), plan_left - s(20), top + s(109)), DT_LEFT);
        let balance = self.account_balance_label();
        let balance_text = balance.as_deref().unwrap_or(if self.reader.checking_account { "正在同步…" } else { "暂不可用" });
        painter.line_text(balance_text, if balance.is_some() { TITLE } else { MEDIUM }, ACCENT_DARK,
            rc(left + s(13), top + s(111), plan_left - s(20), top + s(151)), DT_LEFT);
        let plan = account.and_then(|account| account.plan.as_deref())
            .unwrap_or(if self.reader.checking_account { "正在同步…" } else { "未提供" });
        painter.line_text("当前套餐", SMALL, MUTED, rc(plan_left, top + s(87), inner_right - s(10), top + s(109)), DT_LEFT);
        painter.line_text(plan, MEDIUM, TEXT, rc(plan_left, top + s(111), inner_right - s(10), top + s(151)), DT_LEFT);

        let stale_message;
        let (status, ink) = if self.reader.checking_account {
            ("正在同步 CodeBuddy 账户…", MUTED)
        } else if let Some(error) = self.reader.account_error.as_deref() {
            if balance.is_some() {
                stale_message = format!("已保留上次同步余额。{error}");
                (stale_message.as_str(), ERROR)
            } else {
                (error, ERROR)
            }
        } else if balance.is_some() {
            ("已同步", MUTED)
        } else {
            ("CodeBuddy 暂未返回积分余额", MUTED)
        };
        painter.wrapped_text_aligned(status, SMALL, ink,
            rc(left, top + s(175), inner_right, top + s(212)), DT_LEFT);
        painter.fill(rc(left, top + s(219), inner_right, top + s(220)), LINE);

        let button_top = top + s(229);
        let button_bottom = top + s(259);
        let refresh = (!signing_in && !self.reader.checking_account).then_some(Action::AccountRefresh);
        self.button(painter, rc(left, button_top, left + s(97), button_bottom), "刷新账户", refresh, false);
        let login = (signing_in || !self.reader.checking_models && self.reader.translation.is_none())
            .then_some(Action::AccountLogin);
        let login_label = if signing_in { "取消登录" } else { "重新登录" };
        self.button(painter, rc(left + s(104), button_top, left + s(201), button_bottom), login_label, login, false);
        if signing_in && self.login_link.is_some() {
            self.button(painter, rc(left + s(208), button_top, inner_right, button_bottom),
                "复制登录链接", Some(Action::CopyLoginLink), false);
        }
    }

    fn paint_avatar(&self, painter: &Painter, bounds: RECT, border: u32) {
        let diameter = bounds.right - bounds.left;
        painter.round(bounds, diameter, TINT, border);
        if let Some(avatar) = self.reader.account_avatar.as_ref()
            && avatar.width > 0 && avatar.height > 0
        {
            let inner = rc(bounds.left + 1, bounds.top + 1, bounds.right - 1, bounds.bottom - 1);
            if painter.round_avatar(avatar, inner) {
                return;
            }
        }

        let initial = self.reader.account.as_ref()
            .and_then(|account| account.display_name.as_deref())
            .and_then(|name| name.chars().find(|character| character.is_alphanumeric()));
        if let Some(initial) = initial {
            let text: String = initial.to_uppercase().collect();
            painter.line_text(&text, MEDIUM, ACCENT_DARK, bounds, DT_CENTER);
            return;
        }

        let x = (bounds.left + bounds.right) / 2;
        let y = bounds.top;
        painter.round(rc(x - diameter / 8, y + diameter / 5, x + diameter / 8, y + diameter * 9 / 20),
            diameter / 4, ACCENT, ACCENT);
        painter.round(rc(x - diameter / 4, y + diameter / 2, x + diameter / 4, y + diameter * 4 / 5),
            diameter / 3, ACCENT, ACCENT);
    }
}

impl Painter {
    fn round_avatar(&self, image: &Raster, bounds: RECT) -> bool {
        let flattened = image.bgra.chunks_exact(4).any(|pixel| pixel[3] != 255).then(|| {
            let background = [(TINT >> 16) as u8, (TINT >> 8) as u8, TINT as u8];
            let mut bgra = image.bgra.clone();
            for pixel in bgra.chunks_exact_mut(4) {
                let alpha = u16::from(pixel[3]);
                for (channel, background) in pixel.iter_mut().zip(background) {
                    *channel = ((u16::from(*channel) * alpha + u16::from(background) * (255 - alpha) + 127) / 255) as u8;
                }
                pixel[3] = 255;
            }
            Raster { width: image.width, height: image.height, bgra }
        });
        unsafe {
            let saved = SaveDC(self.dc);
            if saved == 0 {
                return false;
            }
            let region = CreateEllipticRgn(bounds.left, bounds.top, bounds.right, bounds.bottom);
            let success = !region.is_null() && ExtSelectClipRgn(self.dc, region, RGN_AND) != 0;
            if success {
                self.image(flattened.as_ref().unwrap_or(image), bounds);
            }
            RestoreDC(self.dc, saved);
            if !region.is_null() {
                DeleteObject(region);
            }
            success
        }
    }
}
