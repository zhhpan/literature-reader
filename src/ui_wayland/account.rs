use crate::reader::{Raster, Reader};
use crate::translation::format_credit_balance;
use gtk::prelude::*;
use gtk::{gdk, gdk_pixbuf as pixbuf, glib};
use std::cell::Cell;

pub(super) struct AccountPanel {
    pub button: gtk::MenuButton,
    pub popover: gtk::Popover,
    pub refresh: gtk::Button,
    pub login: gtk::Button,
    toolbar_avatar: gtk::Image,
    card_avatar: gtk::Image,
    name: gtk::Label,
    credits: gtk::Label,
    plan: gtk::Label,
    hint: gtk::Label,
    avatar_key: Cell<Option<usize>>,
}

impl AccountPanel {
    pub fn new() -> Self {
        let toolbar_avatar = gtk::Image::from_icon_name("avatar-default-symbolic");
        toolbar_avatar.set_pixel_size(26);
        let button = gtk::MenuButton::builder().child(&toolbar_avatar).build();
        button.set_tooltip_text(Some("CodeBuddy 账户"));

        let contents = gtk::Box::new(gtk::Orientation::Vertical, 13);
        contents.add_css_class("lr-account");
        contents.set_width_request(290);
        let heading = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        let card_avatar = gtk::Image::from_icon_name("avatar-default-symbolic");
        card_avatar.set_pixel_size(48);
        let identity = gtk::Box::new(gtk::Orientation::Vertical, 4);
        let name = text("", "lr-account-name");
        name.set_ellipsize(gtk::pango::EllipsizeMode::End);
        identity.append(&name);
        identity.append(&text("CodeBuddy 账户", "dim-label"));
        heading.append(&card_avatar);
        heading.append(&identity);
        contents.append(&heading);

        let summary = gtk::Box::new(gtk::Orientation::Vertical, 4);
        summary.add_css_class("lr-account-summary");
        let credits = text("", "lr-account-credits");
        let plan = text("", "dim-label");
        summary.append(&text("剩余积分", "dim-label"));
        summary.append(&credits);
        summary.append(&plan);
        contents.append(&summary);
        let hint = text("", "lr-account-hint");
        hint.set_wrap(true);
        contents.append(&hint);

        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let refresh = gtk::Button::with_label("刷新账户");
        let login = gtk::Button::with_label("重新登录");
        actions.append(&refresh);
        actions.append(&login);
        contents.append(&actions);

        let popover = gtk::Popover::new();
        popover.set_child(Some(&contents));
        button.set_popover(Some(&popover));
        Self {
            button, popover, refresh, login, toolbar_avatar, card_avatar, name, credits, plan, hint,
            avatar_key: Cell::new(None),
        }
    }

    pub fn sync(&self, reader: &Reader) {
        self.button.set_visible(reader.signed_in && !reader.signing_in());
        if !reader.signed_in {
            self.sync_avatar(None);
            return;
        }
        self.sync_avatar(reader.account_avatar.as_ref());
        let account = reader.account.as_ref();
        let name = account.and_then(|account| account.display_name.as_deref()).unwrap_or("CodeBuddy");
        self.name.set_text(name);
        self.button.set_tooltip_text(Some(name));
        let credits = account.and_then(|account| format_credit_balance(account.remaining_credits?));
        self.credits.set_text(credits.as_deref().unwrap_or(if reader.checking_account { "同步中…" } else { "暂不可用" }));
        let plan = account.and_then(|account| account.plan.as_deref());
        self.plan.set_text(&plan.map_or_else(String::new, |plan| format!("当前套餐  {plan}")));
        self.plan.set_visible(plan.is_some());

        let hint = if reader.checking_account && credits.is_some() {
            "正在刷新账户…"
        } else if reader.account_error.is_some() && credits.is_some() {
            "刷新失败，当前显示上次同步的余额。"
        } else if let Some(error) = reader.account_error.as_deref() {
            error
        } else if !reader.checking_account && credits.is_none() {
            "CodeBuddy 暂未返回积分余额。"
        } else {
            ""
        };
        self.hint.set_text(hint);
        self.hint.set_visible(!hint.is_empty());
        self.refresh.set_sensitive(!reader.checking_account && !reader.signing_in());
        self.login.set_sensitive(!reader.checking_models && reader.translation.is_none());
    }

    fn sync_avatar(&self, avatar: Option<&Raster>) {
        let key = avatar.map(|image| image.bgra.as_ptr() as usize);
        if self.avatar_key.replace(key) == key {
            return;
        }
        // Set a circular alpha mask before scaling the two avatar sizes.
        let photo = avatar.map(|image| {
            let mut rgba = image.bgra.clone();
            let width = image.width as usize;
            let radius = f64::from(image.width) / 2.0;
            for (position, pixel) in rgba.chunks_exact_mut(4).enumerate() {
                pixel.swap(0, 2);
                let dx = position.rem_euclid(width) as f64 + 0.5 - radius;
                let dy = (position / width) as f64 + 0.5 - radius;
                let coverage = (radius + 0.5 - dx.hypot(dy)).clamp(0.0, 1.0);
                pixel[3] = (f64::from(pixel[3]) * coverage).round() as u8;
            }
            let bytes = glib::Bytes::from_owned(rgba);
            pixbuf::Pixbuf::from_bytes(&bytes, pixbuf::Colorspace::Rgb, true, 8,
                image.width, image.height, image.width * 4)
        });
        for (widget, pixels) in [(&self.toolbar_avatar, 26), (&self.card_avatar, 48)] {
            let resized = photo.as_ref().and_then(|photo| photo.scale_simple(pixels, pixels, pixbuf::InterpType::Bilinear));
            if let Some(resized) = resized {
                widget.set_paintable(Some(&gdk::Texture::for_pixbuf(&resized)));
            } else {
                widget.set_icon_name(Some("avatar-default-symbolic"));
            }
        }
    }
}

fn text(value: &str, class: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(value));
    label.set_xalign(0.0);
    label.add_css_class(class);
    label
}
