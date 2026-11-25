use super::*;

pub(super) enum MousePress {
    Text,
    Action(Action),
}

pub fn run() {
    unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let instance = GetModuleHandleW(null());
        let class = wide("LiteratureReaderWindow");
        let title = wide("文献阅读器");
        let definition = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(window_proc),
            hInstance: instance,
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            lpszClassName: class.as_ptr(),
            ..zeroed()
        };
        if RegisterClassW(&definition) == 0 {
            alert(null_mut(), "无法注册文献阅读器窗口");
            return;
        }
        let dpi = GetDpiForSystem().max(96);
        let mut work: RECT = zeroed();
        SystemParametersInfoW(SPI_GETWORKAREA, 0, (&raw mut work).cast(), 0);
        let width = scale(1240, dpi).min((work.right - work.left - scale(28, dpi)).max(scale(720, dpi)));
        let height = scale(860, dpi).min((work.bottom - work.top - scale(28, dpi)).max(scale(510, dpi)));
        let hwnd = CreateWindowExW(
            0, class.as_ptr(), title.as_ptr(), WS_OVERLAPPEDWINDOW | WS_VSCROLL,
            work.left + (work.right - work.left - width) / 2,
            work.top + (work.bottom - work.top - height) / 2,
            width, height, null_mut(), null_mut(), instance, null(),
        );
        if hwnd.is_null() {
            alert(null_mut(), "无法打开文献阅读器窗口");
            return;
        }
        if let Some(path) = std::env::args_os().nth(1) {
            let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut View;
            (*ptr).open(PathBuf::from(path));
        }
        ShowWindow(hwnd, SW_SHOW);
        UpdateWindow(hwnd);
        let mut msg: MSG = zeroed();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

unsafe extern "system" fn window_proc(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // USERDATA owns the View. Any call that opens a modal window runs outside
    // a borrow of the View because Windows can dispatch another message.
    unsafe {
        if message == WM_NCCREATE {
            let view = Box::into_raw(Box::new(View::new(GetDpiForWindow(hwnd).max(96))));
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, view as isize);
            return DefWindowProcW(hwnd, message, wparam, lparam);
        }
        let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut View;
        if ptr.is_null() {
            return DefWindowProcW(hwnd, message, wparam, lparam);
        }
        match message {
            WM_CREATE => {
                DragAcceptFiles(hwnd, 1);
                SetTimer(hwnd, 1, 45, None);
            }
            WM_DESTROY => {
                KillTimer(hwnd, 1);
                PostQuitMessage(0);
            }
            WM_NCDESTROY => {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                drop(Box::from_raw(ptr));
                return DefWindowProcW(hwnd, message, wparam, lparam);
            }
            WM_GETMINMAXINFO => {
                let dpi = (*ptr).dpi;
                let info = &mut *(lparam as *mut MINMAXINFO);
                info.ptMinTrackSize.x = scale(660, dpi);
                info.ptMinTrackSize.y = scale(455, dpi);
            }
            WM_DPICHANGED => {
                (*ptr).change_dpi((wparam & 0xffff) as u32);
                let area = *(lparam as *const RECT);
                SetWindowPos(hwnd, null_mut(), area.left, area.top, area.right - area.left, area.bottom - area.top,
                    SWP_NOZORDER | SWP_NOACTIVATE);
                redraw(hwnd);
            }
            WM_ERASEBKGND => return 1,
            WM_TIMER => {
                let (changed, login_url) = {
                    let view = &mut *ptr;
                    let changed = view.reader.poll() | view.scroll_text_selection();
                    view.save_reading_position_if_due();
                    (changed, view.reader.take_login_url())
                };
                let opening_login = login_url.is_some();
                if let Some(url) = login_url {
                    let target = wide(&url);
                    let verb = wide("open");
                    if ShellExecuteW(hwnd, verb.as_ptr(), target.as_ptr(), null(), null(), SW_SHOWNORMAL) as isize <= 32
                        && GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut View == ptr
                    {
                        let reader = &mut (*ptr).reader;
                        reader.cancel_login();
                        reader.models_error = Some("无法打开浏览器，请检查 Windows 的默认浏览器设置。".into());
                    }
                }
                if changed || opening_login {
                    redraw(hwnd);
                }
            }
            WM_PAINT => paint_window(hwnd, &mut *ptr),
            WM_SETCURSOR if (lparam & 0xffff) as u32 == HTCLIENT => {
                let mut point: POINT = zeroed();
                if GetCursorPos(&mut point) != 0 && ScreenToClient(hwnd, &mut point) != 0 {
                    let view = &*ptr;
                    let cursor = if view.text_dragging() {
                        Some(IDC_IBEAM)
                    } else if view.hit(point).is_some() {
                        Some(IDC_HAND)
                    } else if view.text_at(point) {
                        Some(IDC_IBEAM)
                    } else {
                        None
                    };
                    if let Some(cursor) = cursor {
                        SetCursor(LoadCursorW(null_mut(), cursor));
                        return 1;
                    }
                }
                return DefWindowProcW(hwnd, message, wparam, lparam);
            }
            WM_MOUSEMOVE => {
                let point = point_from(lparam);
                let view = &mut *ptr;
                let dragging = view.text_dragging();
                let hover = if dragging { None } else { view.hit(point) };
                let changed = (dragging && view.drag_text_selection(point)) || hover != view.hover;
                view.hover = hover;
                if changed {
                    redraw(hwnd);
                }
                if dragging {
                    SetCursor(LoadCursorW(null_mut(), IDC_IBEAM));
                }
                let mut tracking = TRACKMOUSEEVENT {
                    cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE, hwndTrack: hwnd, dwHoverTime: 0,
                };
                TrackMouseEvent(&mut tracking);
            }
            WM_MOUSELEAVE => {
                if (*ptr).hover.take().is_some() {
                    redraw(hwnd);
                }
            }
            WM_LBUTTONDOWN => {
                let point = point_from(lparam);
                let (selecting, changed) = {
                    let view = &mut *ptr;
                    let action = view.hit(point);
                    let selecting = action.is_none() && view.begin_text_selection(point);
                    let changed = selecting || view.clear_text_selection();
                    view.mouse_press = if selecting { Some(MousePress::Text) } else { action.map(MousePress::Action) };
                    (selecting, changed)
                };
                if selecting {
                    SetCapture(hwnd);
                    if GetCapture() != hwnd && GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut View == ptr {
                        (*ptr).cancel_text_drag();
                    }
                }
                if changed {
                    redraw(hwnd);
                }
            }
            WM_LBUTTONUP => {
                let point = point_from(lparam);
                let press = (*ptr).mouse_press.take();
                match press {
                    Some(MousePress::Text) => {
                        (*ptr).finish_text_selection(point);
                        if GetCapture() == hwnd {
                            ReleaseCapture();
                        }
                        redraw(hwnd);
                    }
                    Some(MousePress::Action(action)) if (*ptr).hit(point) == Some(action) => {
                        activate(hwnd, ptr, action);
                    }
                    _ => {}
                }
            }
            WM_CAPTURECHANGED | WM_CANCELMODE => {
                let view = &mut *ptr;
                view.mouse_press = None;
                if view.cancel_text_drag() {
                    redraw(hwnd);
                }
                return DefWindowProcW(hwnd, message, wparam, lparam);
            }
            WM_RBUTTONUP => {
                let point = point_from(lparam);
                if let Some((label, text)) = copy_at(&*ptr, point) {
                    let mut screen = point;
                    ClientToScreen(hwnd, &mut screen);
                    if menu_at(hwnd, screen, &[(1, label.into(), false)]) == 1 {
                        copy_text(hwnd, &text);
                    }
                }
            }
            WM_DROPFILES => {
                if let Some(path) = drop_path(wparam as HDROP) {
                    cancel_mouse_press(hwnd, ptr);
                    (*ptr).open(path);
                    redraw(hwnd);
                }
            }
            WM_MOUSEWHEEL => {
                let view = &mut *ptr;
                let delta = (wparam >> 16) as i16 as i32;
                let mut lines: u32 = 3;
                SystemParametersInfoW(SPI_GETWHEELSCROLLLINES, 0, (&raw mut lines).cast(), 0);
                let pixels = if lines == u32::MAX { view.viewport_height } else { view.s(24) * lines.min(12) as i32 };
                view.scroll(view.scroll - delta * pixels / 120);
                redraw(hwnd);
            }
            WM_VSCROLL => {
                scroll(hwnd, &mut *ptr, (wparam & 0xffff) as i32);
                redraw(hwnd);
            }
            WM_KEYDOWN => {
                let key = wparam as u16;
                let ctrl = GetKeyState(i32::from(VK_CONTROL)) < 0;
                let pdf_view = (*ptr).pdf_view;
                match key {
                    VK_C if ctrl => {
                        if let Some(text) = (*ptr).selected_text() {
                            copy_text(hwnd, &text);
                        }
                        return 0;
                    }
                    VK_A if ctrl => {
                        cancel_mouse_press(hwnd, ptr);
                        if (*ptr).select_all_text() {
                            redraw(hwnd);
                        }
                        return 0;
                    }
                    VK_ESCAPE => {
                        let changed = (*ptr).clear_text_selection();
                        cancel_mouse_press(hwnd, ptr);
                        if changed {
                            redraw(hwnd);
                        }
                        return 0;
                    }
                    _ => {}
                }
                let action = match key {
                    VK_O if ctrl => Some(Action::Open),
                    VK_T if ctrl => Some(Action::TranslateAll),
                    VK_LEFT if pdf_view => Some(Action::Previous),
                    VK_RIGHT if pdf_view => Some(Action::Next),
                    VK_HOME if ctrl && pdf_view => Some(Action::GoTo(1)),
                    VK_END if ctrl && pdf_view => Some(Action::GoTo(usize::MAX)),
                    _ => None,
                };
                if let Some(action) = action {
                    activate(hwnd, ptr, action);
                } else {
                    let scroll_command = match key {
                        VK_UP => SB_LINEUP,
                        VK_DOWN => SB_LINEDOWN,
                        VK_HOME => SB_TOP,
                        VK_END => SB_BOTTOM,
                        VK_PRIOR => SB_PAGEUP,
                        VK_NEXT | VK_SPACE => SB_PAGEDOWN,
                        _ => return DefWindowProcW(hwnd, message, wparam, lparam),
                    };
                    scroll(hwnd, &mut *ptr, scroll_command);
                    redraw(hwnd);
                }
            }
            _ => return DefWindowProcW(hwnd, message, wparam, lparam),
        }
        0
    }
}

fn cancel_mouse_press(hwnd: HWND, ptr: *mut View) {
    unsafe {
        let captured = {
            let view = &mut *ptr;
            view.cancel_text_drag();
            matches!(view.mouse_press.take(), Some(MousePress::Text))
        };
        if captured && GetCapture() == hwnd {
            ReleaseCapture();
        }
    }
}

fn scroll(hwnd: HWND, view: &mut View, command: i32) {
    let position = match command {
        SB_TOP => 0,
        SB_BOTTOM => view.max_scroll,
        SB_LINEUP => view.scroll - view.s(34),
        SB_LINEDOWN => view.scroll + view.s(34),
        SB_PAGEUP => view.scroll - view.viewport_height + view.s(34),
        SB_PAGEDOWN => view.scroll + view.viewport_height - view.s(34),
        SB_THUMBTRACK | SB_THUMBPOSITION => unsafe {
            let mut info = SCROLLINFO { cbSize: size_of::<SCROLLINFO>() as u32, fMask: SIF_TRACKPOS, ..zeroed() };
            GetScrollInfo(hwnd, SB_VERT, &mut info);
            info.nTrackPos
        },
        _ => return,
    };
    view.scroll(position);
}

fn paint_window(hwnd: HWND, view: &mut View) {
    unsafe {
        let mut paint: PAINTSTRUCT = zeroed();
        let target = BeginPaint(hwnd, &mut paint);
        let mut area: RECT = zeroed();
        GetClientRect(hwnd, &mut area);
        if area.right > 0 && area.bottom > 0 {
            let dc = CreateCompatibleDC(target);
            let bitmap = CreateCompatibleBitmap(target, area.right, area.bottom);
            if !dc.is_null() && !bitmap.is_null() {
                let previous = SelectObject(dc, bitmap);
                view.paint(hwnd, dc, area);
                BitBlt(target, 0, 0, area.right, area.bottom, dc, 0, 0, SRCCOPY);
                SelectObject(dc, previous);
            } else {
                view.paint(hwnd, target, area);
            }
            if !bitmap.is_null() {
                DeleteObject(bitmap);
            }
            if !dc.is_null() {
                DeleteDC(dc);
            }
        }
        EndPaint(hwnd, &paint);
    }
}

fn activate(hwnd: HWND, ptr: *mut View, action: Action) {
    unsafe {
        match action {
            Action::Open => {
                cancel_mouse_press(hwnd, ptr);
                if let Some(path) = choose_file(hwnd)
                    && GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut View == ptr
                {
                    (*ptr).open(path);
                }
            }
            Action::OpenRecent(index) => {
                let view = &mut *ptr;
                if !view.reader.loading && view.reader.document.is_none()
                    && let Some(entry) = view.reader.recent_documents.entries().get(index)
                {
                    view.open(entry.path.clone());
                }
            }
            Action::RecentMenu => {
                let (anchor, items, paths) = {
                    let view = &*ptr;
                    let entries = view.reader.recent_documents.entries();
                    let paths: Vec<_> = entries.iter().map(|entry| entry.path.clone()).collect();
                    let items: Vec<_> = entries.iter().enumerate().map(|(index, entry)| {
                        (index as u32 + 1, entry.title.clone(), view.reader.path.as_ref() == Some(&entry.path))
                    }).collect();
                    (menu_anchor(view, action), items, paths)
                };
                let selection = dropdown(hwnd, anchor, &items);
                if GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut View != ptr {
                    return;
                }
                let view = &mut *ptr;
                if let Some(path) = selection.checked_sub(1).and_then(|index| paths.get(index as usize))
                    && (view.reader.path.as_ref() != Some(path) || view.reader.document.is_none())
                {
                    view.open(path.clone());
                }
            }
            Action::Models => {
                let (items, ids, signing_in, anchor) = {
                    let view = &*ptr;
                    let signing_in = view.reader.signing_in();
                    let mut items = vec![(1, if signing_in { "取消登录" } else { "网页登录 CodeBuddy" }.into(), false)];
                    let mut ids = Vec::new();
                    if !signing_in && !view.reader.checking_models
                        && (view.reader.signed_in || view.reader.models_error.is_some())
                    {
                        items.push((2, "刷新可用模型".into(), false));
                    }
                    if !signing_in && view.reader.signed_in {
                        for model in &view.reader.models {
                            let id = ids.len() as u32 + 3;
                            let duplicate_name = view.reader.models.iter().any(|other| other.id != model.id && other.display_name == model.display_name);
                            let label = if duplicate_name {
                                format!("{}  ·  {}", model.display_name, model.id)
                            } else {
                                model.display_name.clone()
                            };
                            items.push((id, label, view.reader.model.as_ref() == Some(&model.id)));
                            ids.push(model.id.clone());
                        }
                    }
                    (items, ids, signing_in, menu_anchor(view, action))
                };
                let selection = dropdown(hwnd, anchor, &items);
                if GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut View != ptr {
                    return;
                }
                let reader = &mut (*ptr).reader;
                match selection {
                    1 if signing_in => reader.cancel_login(),
                    1 => reader.start_login(),
                    2 => reader.refresh_models(),
                    id if id >= 3 => {
                        if let Some(id) = ids.get((id - 3) as usize) {
                            reader.select_model(id);
                        }
                    }
                    _ => {}
                }
            }
            Action::PageMenu => {
                let (anchor, items) = {
                    let view = &*ptr;
                    let Some(doc) = &view.reader.document else { return };
                    let current = view.reader.page;
                    let mut pages: Vec<_> = (current.saturating_sub(5).max(1)..=current.saturating_add(5).min(doc.pages)).collect();
                    pages.extend([1, doc.pages]);
                    pages.sort_unstable();
                    pages.dedup();
                    let items: Vec<_> = pages.into_iter().map(|page| (page as u32, format!("第 {page} 页"), page == current)).collect();
                    (menu_anchor(view, action), items)
                };
                let selection = dropdown(hwnd, anchor, &items);
                if selection > 0 {
                    (*ptr).goto(selection as usize);
                }
            }
            Action::JumpCitation(index) => (*ptr).jump_to_block(index),
            Action::OpenSource { block, link } => {
                let url = {
                    let reader = &(*ptr).reader;
                    reader.reference_links.get(block).and_then(|links| links.get(link))
                        .and_then(|source| url::Url::parse(&source.url).ok())
                };
                if let Some(url) = url.filter(|url| {
                    matches!(url.scheme(), "http" | "https")
                        && url.host_str().is_some()
                        && url.username().is_empty()
                        && url.password().is_none()
                }) {
                    let target = wide(url.as_str());
                    let verb = wide("open");
                    if ShellExecuteW(hwnd, verb.as_ptr(), target.as_ptr(), null(), null(), SW_SHOWNORMAL) as isize <= 32 {
                        alert(hwnd, "无法在默认浏览器打开文献出处。");
                    }
                }
            }
            Action::Details => {
                let view = &*ptr;
                let mut messages = view.reader.document.as_ref().map_or_else(Vec::new, |doc| doc.notices.clone());
                messages.extend(view.reader.load_error.clone());
                messages.extend(view.reader.translation_error.clone());
                messages.extend(view.reader.models_error.clone());
                alert(hwnd, &messages.join("\r\n\r\n"));
            }
            _ => {
                let view = &mut *ptr;
                match action {
                    Action::GoTo(page) => view.goto(page),
                    Action::Previous => view.goto(view.reader.page.saturating_sub(1)),
                    Action::Next => view.goto(view.reader.page + 1),
                    Action::Bilingual | Action::Pdf => view.switch_pdf_view(action == Action::Pdf),
                    Action::TranslateAll => view.reader.start_translation(),
                    Action::Stop => view.reader.cancel_translation(),
                    _ => unreachable!(),
                }
            }
        }
        redraw(hwnd);
    }
}

fn copy_at(view: &View, point: POINT) -> Option<(&'static str, String)> {
    if let Some(text) = view.selected_text_at(point) {
        return Some(("复制选中文字", text));
    }
    let hit = view.copy_targets.iter().find(|item| contains(item.rect, point))?;
    if hit.translation {
        Some(("复制中文翻译", view.reader.translations.get(hit.index)?.as_ref()?.clone()))
    } else {
        Some(("复制原文", view.reader.document.as_ref()?.blocks.get(hit.index)?.text.clone()))
    }
}

fn point_from(lparam: LPARAM) -> POINT {
    POINT { x: lparam as i16 as i32, y: (lparam >> 16) as i16 as i32 }
}

fn menu_anchor(view: &View, action: Action) -> POINT {
    view.hits.iter().find(|hit| hit.action == action)
        .map_or(POINT { x: 0, y: 0 }, |hit| POINT { x: hit.rect.left, y: hit.rect.bottom + 3 })
}

fn dropdown(hwnd: HWND, mut point: POINT, items: &[(u32, String, bool)]) -> u32 {
    unsafe { ClientToScreen(hwnd, &mut point) };
    menu_at(hwnd, point, items)
}

fn menu_at(hwnd: HWND, point: POINT, items: &[(u32, String, bool)]) -> u32 {
    unsafe {
        let menu = CreatePopupMenu();
        if menu.is_null() {
            return 0;
        }
        for (id, label, checked) in items {
            let label = wide(&label.replace('&', "&&"));
            AppendMenuW(menu, MF_STRING | if *checked { MF_CHECKED } else { 0 }, *id as usize, label.as_ptr());
        }
        let selected = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_NONOTIFY,
            point.x, point.y, 0, hwnd, null());
        DestroyMenu(menu);
        selected as u32
    }
}

fn choose_file(hwnd: HWND) -> Option<PathBuf> {
    let mut path = vec![0; 32_768];
    let filter = wide("PDF 文档 (*.pdf)\0*.pdf\0");
    let mut dialog: OPENFILENAMEW = unsafe { zeroed() };
    dialog.lStructSize = size_of::<OPENFILENAMEW>() as u32;
    dialog.hwndOwner = hwnd;
    dialog.lpstrFilter = filter.as_ptr();
    dialog.lpstrFile = path.as_mut_ptr();
    dialog.nMaxFile = path.len() as u32;
    dialog.Flags = OFN_EXPLORER | OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR;
    if unsafe { GetOpenFileNameW(&mut dialog) } == 0 {
        return None;
    }
    let end = path.iter().position(|&item| item == 0)?;
    Some(std::ffi::OsString::from_wide(&path[..end]).into())
}

fn drop_path(handle: HDROP) -> Option<PathBuf> {
    unsafe {
        let len = DragQueryFileW(handle, 0, null_mut(), 0);
        let mut path = vec![0; len as usize + 1];
        let written = DragQueryFileW(handle, 0, path.as_mut_ptr(), len + 1);
        DragFinish(handle);
        (written > 0).then(|| std::ffi::OsString::from_wide(&path[..written as usize]).into())
    }
}

fn copy_text(hwnd: HWND, text: &str) {
    const CF_UNICODETEXT: u32 = 13;
    let text = wide(text);
    unsafe {
        let memory = GlobalAlloc(GMEM_MOVEABLE, text.len() * size_of::<u16>());
        if memory.is_null() { return; }
        let destination = GlobalLock(memory).cast::<u16>();
        if destination.is_null() {
            GlobalFree(memory);
            return;
        }
        std::ptr::copy_nonoverlapping(text.as_ptr(), destination, text.len());
        GlobalUnlock(memory);
        if OpenClipboard(hwnd) == 0 {
            GlobalFree(memory);
            return;
        }
        EmptyClipboard();
        if SetClipboardData(CF_UNICODETEXT, memory).is_null() {
            GlobalFree(memory);
        }
        CloseClipboard();
    }
}

fn alert(hwnd: HWND, message: &str) {
    unsafe { MessageBoxW(hwnd, wide(message).as_ptr(), wide("文献阅读器").as_ptr(), MB_OK | MB_ICONINFORMATION) };
}

fn redraw(hwnd: HWND) {
    unsafe { InvalidateRect(hwnd, null(), 0) };
}
