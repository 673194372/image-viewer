use gdk4::prelude::*;
use glib::clone;
use gtk4::prelude::*;
use gtk4::{gdk, gio, glib, Application, ApplicationWindow, Box, Button, DrawingArea, FileDialog, Label, Orientation, CssProvider, cairo};
use gtk4_layer_shell::{Layer, LayerShell, Edge};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::env;

const APP_ID: &str = "com.github.image-viewer";
const TITLEBAR_HEIGHT: i32 = 28;
const MIN_WIN_WIDTH: i32 = 400;
const MIN_WIN_HEIGHT: i32 = 300;

#[derive(Clone, Copy, PartialEq)]
enum WindowMode {
    Normal,
    Overlay,
}

// 获取屏幕可用尺寸
fn get_screen_size() -> (i32, i32) {
    if let Some(display) = gdk::Display::default() {
        if let Some(monitor) = display.monitors().item(0) {
            if let Some(monitor) = monitor.downcast_ref::<gdk::Monitor>() {
                let geom = monitor.geometry();
                return (geom.width(), geom.height());
            }
        }
    }
    (1920, 1080) // fallback
}

// 计算目标窗口大小
fn calc_target_size(img_w: i32, img_h: i32) -> (i32, i32) {
    let (screen_w, screen_h) = get_screen_size();
    let max_w = screen_w - 100; // 留边距
    let max_h = screen_h - 100;
    let w = img_w.clamp(MIN_WIN_WIDTH, max_w);
    let h = (img_h + TITLEBAR_HEIGHT).clamp(MIN_WIN_HEIGHT, max_h);
    (w, h)
}

fn print_help() {
    eprintln!("Usage: image-viewer [OPTIONS] [FILE]");
    eprintln!();
    eprintln!("Options:");
    eprintln!("  -o, --overlay    Start in overlay (always-on-top) mode");
    eprintln!("  -h, --help       Show this help message");
    eprintln!("  -v, --version    Show version");
}

fn main() -> glib::ExitCode {
    // 解析命令行参数
    let args: Vec<String> = env::args().collect();
    let mut start_overlay = false;
    let mut file_path: Option<String> = None;
    
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "-o" | "--overlay" => start_overlay = true,
            "-h" | "--help" => {
                print_help();
                return glib::ExitCode::SUCCESS;
            }
            "-v" | "--version" => {
                eprintln!("image-viewer {}", env!("CARGO_PKG_VERSION"));
                return glib::ExitCode::SUCCESS;
            }
            arg if !arg.starts_with('-') => {
                file_path = Some(arg.to_string());
            }
            _ => {
                eprintln!("Unknown option: {}", args[i]);
                print_help();
                return glib::ExitCode::from(1);
            }
        }
        i += 1;
    }
    
    let app = Application::builder()
        .application_id(APP_ID)
        .flags(gio::ApplicationFlags::HANDLES_OPEN)
        .build();
    
    let initial_file: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(file_path));
    let initial_mode: Rc<Cell<WindowMode>> = Rc::new(Cell::new(
        if start_overlay { WindowMode::Overlay } else { WindowMode::Normal }
    ));
    
    let initial_file_open = initial_file.clone();
    app.connect_open(move |app, files, _| {
        if let Some(file) = files.first() {
            if let Some(path) = file.path() {
                *initial_file_open.borrow_mut() = Some(path.to_string_lossy().to_string());
            }
        }
        app.activate();
    });

    let initial_file_activate = initial_file.clone();
    let initial_mode_activate = initial_mode.clone();
    app.connect_activate(move |app| {
        build_ui(app, initial_file_activate.borrow_mut().take(), initial_mode_activate.get());
    });
    
    // 使用空参数运行，避免 GTK 解析我们的自定义参数
    app.run_with_args::<&str>(&[])
}

struct ImageState {
    pixbuf: Option<gdk::Texture>,
    scale: f64,
    offset_x: f64,
    offset_y: f64,
    rotation: i32,
    original_width: i32,
    original_height: i32,
}

// 置顶模式下的窗口位置（layer-shell 使用 margin 定位）
struct OverlayPosition {
    margin_left: i32,
    margin_top: i32,
}

impl Default for ImageState {
    fn default() -> Self {
        Self { pixbuf: None, scale: 1.0, offset_x: 0.0, offset_y: 0.0, rotation: 0,
               original_width: 0, original_height: 0 }
    }
}

impl Default for OverlayPosition {
    fn default() -> Self {
        Self { margin_left: 100, margin_top: 100 }
    }
}

// 更新窗口大小的核心函数
// 强制窗口自适应（Snap-to-fit）
fn update_window_size(win: &ApplicationWindow, da: &DrawingArea, scaled_w: i32, scaled_h: i32) {
    let (target_w, target_h) = calc_target_size(scaled_w, scaled_h);
    let content_h = target_h - TITLEBAR_HEIGHT;
    
    // 利用 resizable 副作用强制窗口收缩
    win.set_resizable(false);
    da.set_content_width(target_w);
    da.set_content_height(content_h);
    win.set_default_size(target_w, target_h);
    win.set_resizable(true);
}

// 检查图片是否触发屏幕边缘限制
fn is_at_screen_limit(scaled_w: i32, scaled_h: i32) -> bool {
    let (screen_w, screen_h) = get_screen_size();
    let max_w = screen_w - 100;
    let max_h = screen_h - 100 - TITLEBAR_HEIGHT;
    scaled_w >= max_w || scaled_h >= max_h
}

// 计算旋转后的图片尺寸
fn get_rotated_size(state: &ImageState) -> (i32, i32) {
    match state.rotation % 2 {
        0 => (state.original_width, state.original_height),
        _ => (state.original_height, state.original_width),
    }
}

// 获取缩放后的图片尺寸
fn get_scaled_size(state: &ImageState) -> (i32, i32) {
    let (w, h) = get_rotated_size(state);
    ((w as f64 * state.scale) as i32, (h as f64 * state.scale) as i32)
}

// 创建绘图区域的绘制函数
fn create_draw_func(
    state: Rc<RefCell<ImageState>>,
    cached_surface: Rc<RefCell<Option<cairo::ImageSurface>>>,
    cached_rotation: Rc<Cell<i32>>,
    is_overlay: bool,
) -> impl Fn(&DrawingArea, &cairo::Context, i32, i32) {
    move |_, cr, width, height| {
        let state = state.borrow();
        
        // 置顶模式使用透明背景
        if is_overlay {
            cr.set_operator(cairo::Operator::Source);
            cr.set_source_rgba(0.0, 0.0, 0.0, 0.0);
            cr.paint().ok();
            cr.set_operator(cairo::Operator::Over);
        } else {
            cr.set_source_rgb(0.12, 0.12, 0.12);
            cr.paint().ok();
        }
        
        if let Some(ref texture) = state.pixbuf {
            let need_update = cached_rotation.get() != state.rotation || cached_surface.borrow().is_none();
            if need_update {
                let (tw, th) = (texture.width(), texture.height());
                if let Ok(surface) = cairo::ImageSurface::create(cairo::Format::ARgb32, tw, th) {
                    let snapshot = gtk4::Snapshot::new();
                    texture.snapshot(&snapshot, tw as f64, th as f64);
                    if let Some(node) = snapshot.to_node() {
                        if let Ok(ctx) = cairo::Context::new(&surface) {
                            node.draw(&ctx);
                        }
                    }
                    *cached_surface.borrow_mut() = Some(surface);
                    cached_rotation.set(state.rotation);
                }
            }
            
            if let Some(ref surface) = *cached_surface.borrow() {
                let (img_w, img_h) = get_rotated_size(&state);
                let scaled_w = img_w as f64 * state.scale;
                let scaled_h = img_h as f64 * state.scale;
                
                // 置顶模式：图片填满窗口；普通模式：居中+偏移
                let (x, y) = if is_overlay {
                    (0.0, 0.0)
                } else {
                    ((width as f64 - scaled_w) / 2.0 + state.offset_x,
                     (height as f64 - scaled_h) / 2.0 + state.offset_y)
                };
                
                cr.save().ok();
                cr.translate(x + scaled_w / 2.0, y + scaled_h / 2.0);
                cr.rotate(state.rotation as f64 * std::f64::consts::FRAC_PI_2);
                cr.scale(state.scale, state.scale);
                cr.translate(-state.original_width as f64 / 2.0, -state.original_height as f64 / 2.0);
                cr.set_source_surface(surface, 0.0, 0.0).ok();
                cr.source().set_filter(cairo::Filter::Bilinear);
                cr.paint().ok();
                cr.restore().ok();
            }
        }
    }
}

// 创建置顶模式窗口
fn create_overlay_window(
    app: &Application,
    state: Rc<RefCell<ImageState>>,
    overlay_pos: Rc<RefCell<OverlayPosition>>,
    on_exit_overlay: impl Fn() + 'static,
) -> ApplicationWindow {
    let (scaled_w, scaled_h) = {
        let s = state.borrow();
        get_scaled_size(&s)
    };
    
    let window = ApplicationWindow::builder()
        .application(app)
        .decorated(false)
        .default_width(scaled_w.max(50))
        .default_height(scaled_h.max(50))
        .build();
    
    // 初始化 layer-shell
    window.init_layer_shell();
    window.set_layer(Layer::Overlay);
    window.set_keyboard_mode(gtk4_layer_shell::KeyboardMode::None);
    
    // 设置锚点和边距定位窗口
    window.set_anchor(Edge::Left, true);
    window.set_anchor(Edge::Top, true);
    window.set_anchor(Edge::Right, false);
    window.set_anchor(Edge::Bottom, false);
    
    {
        let pos = overlay_pos.borrow();
        window.set_margin(Edge::Left, pos.margin_left);
        window.set_margin(Edge::Top, pos.margin_top);
    }
    
    // 创建绘图区域
    let drawing_area = DrawingArea::new();
    drawing_area.set_content_width(scaled_w.max(50));
    drawing_area.set_content_height(scaled_h.max(50));
    
    let cached_surface: Rc<RefCell<Option<cairo::ImageSurface>>> = Rc::new(RefCell::new(None));
    let cached_rotation: Rc<Cell<i32>> = Rc::new(Cell::new(-1));
    
    let draw_func = create_draw_func(state.clone(), cached_surface.clone(), cached_rotation.clone(), true);
    drawing_area.set_draw_func(draw_func);
    
    window.set_child(Some(&drawing_area));
    
    // 滚轮缩放
    let scroll_ctrl = gtk4::EventControllerScroll::new(gtk4::EventControllerScrollFlags::VERTICAL);
    let state_scroll = state.clone();
    let da_scroll = drawing_area.clone();
    let win_scroll = window.clone();
    scroll_ctrl.connect_scroll(move |_, _, dy| {
        let mut s = state_scroll.borrow_mut();
        if s.pixbuf.is_none() { return glib::Propagation::Proceed; }
        
        let factor = if dy < 0.0 { 1.1 } else { 1.0 / 1.1 };
        s.scale = (s.scale * factor).clamp(0.1, 50.0);
        
        let (scaled_w, scaled_h) = get_scaled_size(&s);
        drop(s);
        
        // 更新窗口和绘图区大小
        da_scroll.set_content_width(scaled_w.max(50));
        da_scroll.set_content_height(scaled_h.max(50));
        win_scroll.set_default_size(scaled_w.max(50), scaled_h.max(50));
        da_scroll.queue_draw();
        
        glib::Propagation::Stop
    });
    drawing_area.add_controller(scroll_ctrl);
    
    // 拖动窗口（移动位置）
    let drag_ctrl = gtk4::GestureDrag::builder().button(1).build();
    let win_drag = window.clone();
    let overlay_pos_drag = overlay_pos.clone();
    let drag_start_pos = Rc::new(Cell::new((0i32, 0i32)));
    let drag_start_clone = drag_start_pos.clone();
    
    drag_ctrl.connect_drag_begin(clone!(#[strong] overlay_pos_drag, move |_, _, _| {
        let pos = overlay_pos_drag.borrow();
        drag_start_clone.set((pos.margin_left, pos.margin_top));
    }));
    
    drag_ctrl.connect_drag_update(clone!(#[strong] overlay_pos_drag, #[strong] win_drag, #[strong] drag_start_pos,
        move |_, dx, dy| {
            let (start_left, start_top) = drag_start_pos.get();
            let new_left = (start_left as f64 + dx) as i32;
            let new_top = (start_top as f64 + dy) as i32;
            
            {
                let mut pos = overlay_pos_drag.borrow_mut();
                pos.margin_left = new_left.max(0);
                pos.margin_top = new_top.max(0);
            }
            
            win_drag.set_margin(Edge::Left, new_left.max(0));
            win_drag.set_margin(Edge::Top, new_top.max(0));
        }
    ));
    drawing_area.add_controller(drag_ctrl);
    
    // 双击退出置顶模式
    let double_click = gtk4::GestureClick::builder().button(1).build();
    let on_exit = Rc::new(on_exit_overlay);
    let on_exit_dbl = on_exit.clone();
    let win_dbl = window.clone();
    double_click.connect_pressed(move |gesture, n_press, _, _| {
        if n_press == 2 {
            gesture.set_state(gtk4::EventSequenceState::Claimed);
            win_dbl.close();
            on_exit_dbl();
        }
    });
    drawing_area.add_controller(double_click);
    
    // 右键关闭
    let right_click = gtk4::GestureClick::builder().button(3).build();
    let win_right = window.clone();
    right_click.connect_pressed(move |_, _, _, _| {
        win_right.close();
        on_exit();
    });
    drawing_area.add_controller(right_click);
    
    window
}

fn build_ui(app: &Application, initial_path: Option<String>, initial_mode: WindowMode) {
    let state = Rc::new(RefCell::new(ImageState::default()));
    let mouse_pos = Rc::new(Cell::new((0.0f64, 0.0f64)));
    let current_mode = Rc::new(Cell::new(initial_mode));
    let overlay_pos = Rc::new(RefCell::new(OverlayPosition::default()));
    let overlay_window: Rc<RefCell<Option<ApplicationWindow>>> = Rc::new(RefCell::new(None));
    
    // 预读图片尺寸
    let (init_img_w, init_img_h) = if let Some(ref path) = initial_path {
        if let Ok(texture) = gdk::Texture::from_filename(path) {
            (texture.width(), texture.height())
        } else { (800, 600) }
    } else { (800, 600) };
    let (init_w, init_h) = calc_target_size(init_img_w, init_img_h);

    // 加载 CSS (GTK4 兼容语法)
    let css = CssProvider::new();
    css.load_from_string(r#"
        .titlebar { 
            background-color: #323232;
            padding: 0 6px;
            border-bottom: 1px solid #1a1a1a;
        }
        .titlebar-btn { 
            min-height: 26px; 
            min-width: 32px; 
            padding: 4px 8px; 
            margin: 2px 2px;
            background: none;
            background-color: transparent; 
            border: none; 
            border-radius: 6px;
            color: #909090;
            box-shadow: none;
        }
        .titlebar-btn:hover { 
            background-color: #4a4a4a;
            color: #ffffff;
        }
        .titlebar-btn:active {
            background-color: #3a3a3a;
        }
        .close-btn:hover { 
            background-color: #e81123; 
            color: #ffffff; 
        }
        .close-btn:active {
            background-color: #c41020;
        }
        .path-label { 
            color: #707070; 
            font-size: 11px; 
            margin: 0 12px;
        }
        .info-label { 
            color: #909090; 
            font-size: 10px; 
            margin: 0 8px;
            padding: 3px 8px;
            background-color: #3a3a3a;
            border-radius: 4px;
        }
    "#);
    gtk4::style_context_add_provider_for_display(
        &gdk::Display::default().unwrap(), &css, gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );

    let drawing_area = DrawingArea::new();
    drawing_area.set_hexpand(true);
    drawing_area.set_vexpand(true);

    // 绘制回调 - 缓存原始 surface，使用 cairo 变换实现缩放
    let state_draw = state.clone();
    let cached_surface: Rc<RefCell<Option<cairo::ImageSurface>>> = Rc::new(RefCell::new(None));
    let cached_rotation: Rc<Cell<i32>> = Rc::new(Cell::new(-1));
    let cs = cached_surface.clone();
    let cr_rot = cached_rotation.clone();
    
    drawing_area.set_draw_func(move |_, cr, width, height| {
        let state = state_draw.borrow();
        cr.set_source_rgb(0.12, 0.12, 0.12);
        cr.paint().ok();
        
        if let Some(ref texture) = state.pixbuf {
            // 只在旋转变化或首次加载时重新生成原始 surface
            let need_update = cached_rotation.get() != state.rotation || cached_surface.borrow().is_none();
            if need_update {
                let (tw, th) = (texture.width(), texture.height());
                if let Ok(surface) = cairo::ImageSurface::create(cairo::Format::ARgb32, tw, th) {
                    let snapshot = gtk4::Snapshot::new();
                    texture.snapshot(&snapshot, tw as f64, th as f64);
                    if let Some(node) = snapshot.to_node() {
                        if let Ok(ctx) = cairo::Context::new(&surface) {
                            node.draw(&ctx);
                        }
                    }
                    *cached_surface.borrow_mut() = Some(surface);
                    cached_rotation.set(state.rotation);
                }
            }
            
            if let Some(ref surface) = *cached_surface.borrow() {
                let (img_w, img_h) = match state.rotation % 2 {
                    0 => (state.original_width as f64, state.original_height as f64),
                    _ => (state.original_height as f64, state.original_width as f64),
                };
                let scaled_w = img_w * state.scale;
                let scaled_h = img_h * state.scale;
                let x = (width as f64 - scaled_w) / 2.0 + state.offset_x;
                let y = (height as f64 - scaled_h) / 2.0 + state.offset_y;
                
                cr.save().ok();
                // 使用快速滤波器提升性能
                cr.translate(x + scaled_w / 2.0, y + scaled_h / 2.0);
                cr.rotate(state.rotation as f64 * std::f64::consts::FRAC_PI_2);
                cr.scale(state.scale, state.scale);
                cr.translate(-state.original_width as f64 / 2.0, -state.original_height as f64 / 2.0);
                cr.set_source_surface(surface, 0.0, 0.0).ok();
                // 使用双线性滤波保持图片质量
                cr.source().set_filter(cairo::Filter::Bilinear);
                cr.paint().ok();
                cr.restore().ok();
            }
        }
    });

    // 窗口和标签引用
    let zoom_label_ref: Rc<RefCell<Option<Label>>> = Rc::new(RefCell::new(None));
    let window_ref: Rc<RefCell<Option<ApplicationWindow>>> = Rc::new(RefCell::new(None));
    let da_ref: Rc<RefCell<Option<DrawingArea>>> = Rc::new(RefCell::new(None));
    
    let zoom_lbl = zoom_label_ref.clone();
    let win_scroll = window_ref.clone();
    let da_scroll_ref = da_ref.clone();
    
    // 鼠标滚轮缩放
    let scroll_ctrl = gtk4::EventControllerScroll::new(gtk4::EventControllerScrollFlags::VERTICAL);
    let state_scroll = state.clone();
    let da_scroll = drawing_area.clone();
    let mouse_scroll = mouse_pos.clone();
    scroll_ctrl.connect_scroll(move |_, _, dy| {
        let mut state = state_scroll.borrow_mut();
        if state.pixbuf.is_none() { return glib::Propagation::Proceed; }
        
        let (mx, my) = mouse_scroll.get();
        let (width, height) = (da_scroll.width() as f64, da_scroll.height() as f64);
        let old_scale = state.scale;
        let factor = if dy < 0.0 { 1.1 } else { 1.0 / 1.1 };
        state.scale = (state.scale * factor).clamp(0.1, 50.0);
        
        let (img_w, img_h) = match state.rotation % 2 {
            0 => (state.original_width as f64, state.original_height as f64),
            _ => (state.original_height as f64, state.original_width as f64),
        };
        let scaled_w = (img_w * state.scale) as i32;
        let scaled_h = (img_h * state.scale) as i32;
        
        // 检查是否触发屏幕边缘限制
        let at_limit = is_at_screen_limit(scaled_w, scaled_h);
        
        // 以鼠标位置为中心缩放（仅当图片大于窗口时）
        if at_limit {
            let (cx, cy) = (width / 2.0 + state.offset_x, height / 2.0 + state.offset_y);
            let ratio = state.scale / old_scale;
            state.offset_x += (mx - cx) * (1.0 - ratio);
            state.offset_y += (my - cy) * (1.0 - ratio);
        } else {
            // 图片小于屏幕，居中显示
            state.offset_x = 0.0;
            state.offset_y = 0.0;
        }
        
        // 更新缩放率标签
        if let Some(ref lbl) = *zoom_lbl.borrow() {
            lbl.set_text(&format!("{:.0}%", state.scale * 100.0));
        }
        
        // 调整窗口大小（仅当图片未触发屏幕限制时才强制调整）
        if let (Some(win), Some(da)) = (&*win_scroll.borrow(), &*da_scroll_ref.borrow()) {
            if !at_limit {
                // 图片小于屏幕，强制窗口收缩到图片大小
                update_window_size(win, da, scaled_w, scaled_h);
            } else {
                // 图片大于屏幕，只更新内容大小，不强制调整窗口
                let (target_w, target_h) = calc_target_size(scaled_w, scaled_h);
                da.set_content_width(target_w);
                da.set_content_height(target_h - TITLEBAR_HEIGHT);
            }
        }
        
        da_scroll.queue_draw();
        glib::Propagation::Stop
    });
    drawing_area.add_controller(scroll_ctrl);

    // 追踪鼠标位置
    let motion_ctrl = gtk4::EventControllerMotion::new();
    let mouse_motion = mouse_pos.clone();
    motion_ctrl.connect_motion(move |_, x, y| { mouse_motion.set((x, y)); });
    drawing_area.add_controller(motion_ctrl);

    // 拖拽移动图片
    let drag_ctrl = gtk4::GestureDrag::builder().button(1).build();
    let state_drag = state.clone();
    let da_drag = drawing_area.clone();
    let drag_start = Rc::new(Cell::new((0.0f64, 0.0f64)));
    let drag_start_clone = drag_start.clone();
    drag_ctrl.connect_drag_begin(clone!(#[strong] state_drag, move |_, _, _| {
        let s = state_drag.borrow();
        drag_start_clone.set((s.offset_x, s.offset_y));
    }));
    drag_ctrl.connect_drag_update(clone!(#[strong] state_drag, #[strong] da_drag, #[strong] drag_start,
        move |_, dx, dy| {
            let mut s = state_drag.borrow_mut();
            let (sx, sy) = drag_start.get();
            s.offset_x = sx + dx;
            s.offset_y = sy + dy;
            da_drag.queue_draw();
        }
    ));
    drawing_area.add_controller(drag_ctrl);

    // 双击进入置顶模式
    let double_click_ctrl = gtk4::GestureClick::builder().button(1).build();
    let state_dblclick = state.clone();
    let mode_dblclick = current_mode.clone();
    let overlay_pos_dblclick = overlay_pos.clone();
    let overlay_win_dblclick = overlay_window.clone();
    let window_ref_dblclick = window_ref.clone();
    let da_ref_dblclick = da_ref.clone();
    let app_dblclick = app.clone();
    
    double_click_ctrl.connect_pressed(move |gesture, n_press, _, _| {
        if n_press == 2 && state_dblclick.borrow().pixbuf.is_some() {
            gesture.set_state(gtk4::EventSequenceState::Claimed);
            mode_dblclick.set(WindowMode::Overlay);
            
            // 计算图片在屏幕上的位置
            // 使用双击点作为参考：双击点相对于图片的位置在切换后应保持不变
            if let Some(ref da) = *da_ref_dblclick.borrow() {
                if let Some(ref win) = *window_ref_dblclick.borrow() {
                    let s = state_dblclick.borrow();
                    let (scaled_w, scaled_h) = get_scaled_size(&s);
                    let da_w = da.width() as f64;
                    let da_h = da.height() as f64;
                    
                    // 图片在 drawing_area 中的位置
                    let img_x_in_da = (da_w - scaled_w as f64) / 2.0 + s.offset_x;
                    let img_y_in_da = (da_h - scaled_h as f64) / 2.0 + s.offset_y;
                    
                    // drawing_area 在窗口内的 y 偏移 = 标题栏高度
                    let da_y_in_win = TITLEBAR_HEIGHT as f64;
                    
                    // 计算 overlay 的 margin，使图片在屏幕上位置不变
                    // Wayland 下无法获取窗口绝对位置，假设窗口大致居中
                    let (screen_w, screen_h) = get_screen_size();
                    let win_w = win.width();
                    let win_h = win.height();
                    
                    // 假设窗口居中，计算图片应该在的屏幕位置
                    let approx_win_x = (screen_w - win_w) / 2;
                    let approx_win_y = (screen_h - win_h) / 2;
                    let margin_left = approx_win_x + (img_x_in_da as i32);
                    let margin_top = approx_win_y + (da_y_in_win as i32) + (img_y_in_da as i32);
                    
                    // 更新 overlay 位置
                    {
                        let mut pos = overlay_pos_dblclick.borrow_mut();
                        pos.margin_left = margin_left.max(0);
                        pos.margin_top = margin_top.max(0);
                    }
                    drop(s);
                }
            }
            
            // 隐藏普通窗口
            if let Some(ref win) = *window_ref_dblclick.borrow() {
                win.set_visible(false);
            }
            
            // 创建置顶窗口
            let mode_exit = mode_dblclick.clone();
            let win_ref_exit = window_ref_dblclick.clone();
            let overlay_win_exit = overlay_win_dblclick.clone();
            let state_exit = state_dblclick.clone();
            let da_ref_exit = da_ref_dblclick.clone();
            
            let overlay = create_overlay_window(
                &app_dblclick,
                state_dblclick.clone(),
                overlay_pos_dblclick.clone(),
                move || {
                    mode_exit.set(WindowMode::Normal);
                    
                    // 退出时重置 offset，让普通窗口中图片居中
                    {
                        let mut s = state_exit.borrow_mut();
                        s.offset_x = 0.0;
                        s.offset_y = 0.0;
                    }
                    
                    // 显示普通窗口
                    if let Some(ref win) = *win_ref_exit.borrow() {
                        win.set_visible(true);
                        win.present();
                        // 触发重绘
                        if let Some(ref da) = *da_ref_exit.borrow() {
                            da.queue_draw();
                        }
                    }
                    *overlay_win_exit.borrow_mut() = None;
                },
            );
            overlay.present();
            *overlay_win_dblclick.borrow_mut() = Some(overlay);
        }
    });
    drawing_area.add_controller(double_click_ctrl);

    // 自定义标题栏
    let titlebar = Box::new(Orientation::Horizontal, 2);
    titlebar.add_css_class("titlebar");
    
    let open_btn = Button::builder().icon_name("document-open-symbolic").tooltip_text("打开").build();
    open_btn.add_css_class("titlebar-btn");
    open_btn.add_css_class("flat");
    
    let reset_btn = Button::builder().icon_name("zoom-fit-best-symbolic").tooltip_text("恢复").build();
    reset_btn.add_css_class("titlebar-btn");
    reset_btn.add_css_class("flat");
    
    let rotate_btn = Button::builder().icon_name("object-rotate-right-symbolic").tooltip_text("旋转").build();
    rotate_btn.add_css_class("titlebar-btn");
    rotate_btn.add_css_class("flat");
    
    let copy_btn = Button::builder().icon_name("edit-copy-symbolic").tooltip_text("复制").build();
    copy_btn.add_css_class("titlebar-btn");
    copy_btn.add_css_class("flat");
    
    let close_btn = Button::builder().icon_name("window-close-symbolic").tooltip_text("关闭").build();
    close_btn.add_css_class("titlebar-btn");
    close_btn.add_css_class("close-btn");
    close_btn.add_css_class("flat");
    
    let path_label = Label::new(None);
    path_label.add_css_class("path-label");
    path_label.set_ellipsize(gtk4::pango::EllipsizeMode::Middle);
    path_label.set_hexpand(true);
    path_label.set_halign(gtk4::Align::Start);
    
    let drag_area = gtk4::WindowHandle::new();
    drag_area.set_hexpand(true);
    drag_area.set_child(Some(&path_label));
    
    let zoom_label = Label::new(Some("100%"));
    zoom_label.add_css_class("info-label");
    zoom_label.set_tooltip_text(Some("缩放率"));
    *zoom_label_ref.borrow_mut() = Some(zoom_label.clone());
    
    let res_label = Label::new(None);
    res_label.add_css_class("info-label");
    res_label.set_tooltip_text(Some("分辨率"));
    
    titlebar.append(&open_btn);
    titlebar.append(&reset_btn);
    titlebar.append(&rotate_btn);
    titlebar.append(&copy_btn);
    titlebar.append(&drag_area);
    titlebar.append(&res_label);
    titlebar.append(&zoom_label);
    titlebar.append(&close_btn);

    let content = Box::new(Orientation::Vertical, 0);
    content.append(&titlebar);
    content.append(&drawing_area);

    let window = ApplicationWindow::builder()
        .application(app)
        .decorated(false)
        .resizable(true)
        .default_width(init_w)
        .default_height(init_h)
        .child(&content)
        .build();
    
    // 初始设置内容大小
    drawing_area.set_content_width(init_w);
    drawing_area.set_content_height(init_h - TITLEBAR_HEIGHT);
    
    *window_ref.borrow_mut() = Some(window.clone());
    *da_ref.borrow_mut() = Some(drawing_area.clone());
    
    // 边缘拖动调整窗口大小
    const EDGE_SIZE: f64 = 8.0;
    let win_resize = window.clone();
    let resize_motion = gtk4::EventControllerMotion::new();
    resize_motion.connect_motion(clone!(#[strong] win_resize, move |ctrl, x, y| {
        if let Some(widget) = ctrl.widget() {
            let (w, h) = (widget.width() as f64, widget.height() as f64);
            let (on_l, on_r, on_t, on_b) = (x < EDGE_SIZE, x > w - EDGE_SIZE, y < EDGE_SIZE, y > h - EDGE_SIZE);
            let cursor = match (on_l, on_r, on_t, on_b) {
                (true, _, true, _) => Some("nw-resize"), (true, _, _, true) => Some("sw-resize"),
                (_, true, true, _) => Some("ne-resize"), (_, true, _, true) => Some("se-resize"),
                (true, _, _, _) => Some("w-resize"), (_, true, _, _) => Some("e-resize"),
                (_, _, true, _) => Some("n-resize"), (_, _, _, true) => Some("s-resize"),
                _ => None,
            };
            if let Some(name) = cursor { win_resize.set_cursor_from_name(Some(name)); }
            else { win_resize.set_cursor(None); }
        }
    }));
    
    let win_resize_drag = window.clone();
    let resize_gesture = gtk4::GestureDrag::builder().button(1).build();
    resize_gesture.connect_drag_begin(clone!(#[strong] win_resize_drag, move |gesture, x, y| {
        if let Some(widget) = gesture.widget() {
            let (w, h) = (widget.width() as f64, widget.height() as f64);
            let (on_l, on_r, on_t, on_b) = (x < EDGE_SIZE, x > w - EDGE_SIZE, y < EDGE_SIZE, y > h - EDGE_SIZE);
            let edge = match (on_l, on_r, on_t, on_b) {
                (true, _, true, _) => Some(gdk::SurfaceEdge::NorthWest), (true, _, _, true) => Some(gdk::SurfaceEdge::SouthWest),
                (_, true, true, _) => Some(gdk::SurfaceEdge::NorthEast), (_, true, _, true) => Some(gdk::SurfaceEdge::SouthEast),
                (true, _, _, _) => Some(gdk::SurfaceEdge::West), (_, true, _, _) => Some(gdk::SurfaceEdge::East),
                (_, _, true, _) => Some(gdk::SurfaceEdge::North), (_, _, _, true) => Some(gdk::SurfaceEdge::South),
                _ => None,
            };
            if let Some(edge) = edge {
                if let Some(native) = win_resize_drag.native() {
                    if let Some(surface) = native.surface() {
                        if let Some(toplevel) = surface.downcast_ref::<gdk::Toplevel>() {
                            gesture.set_state(gtk4::EventSequenceState::Claimed);
                            toplevel.begin_resize(edge, gesture.device().as_ref(), 1, x, y, gdk::CURRENT_TIME);
                        }
                    }
                }
            }
        }
    }));
    content.add_controller(resize_motion);
    content.add_controller(resize_gesture);

    let win_close = window.clone();
    close_btn.connect_clicked(move |_| { win_close.close(); });

    // 加载图片函数
    let path_lbl = path_label.clone();
    let zoom_lbl = zoom_label.clone();
    let res_lbl = res_label.clone();
    let win_load = window_ref.clone();
    let da_load = da_ref.clone();
    let load_image = {
        let state = state.clone();
        let da = drawing_area.clone();
        let cs = cs.clone();
        let cr_rot = cr_rot.clone();
        Rc::new(move |path: &str| {
            match gdk::Texture::from_filename(path) {
                Ok(texture) => {
                    let mut s = state.borrow_mut();
                    s.original_width = texture.width();
                    s.original_height = texture.height();
                    s.pixbuf = Some(texture);
                    s.scale = 1.0;
                    s.offset_x = 0.0;
                    s.offset_y = 0.0;
                    s.rotation = 0;
                    
                    // 计算适应窗口的缩放
                    let (target_w, target_h) = calc_target_size(s.original_width, s.original_height);
                    let content_h = target_h - TITLEBAR_HEIGHT;
                    s.scale = (target_w as f64 / s.original_width as f64)
                        .min(content_h as f64 / s.original_height as f64)
                        .min(1.0);
                    
                    let scaled_w = (s.original_width as f64 * s.scale) as i32;
                    let scaled_h = (s.original_height as f64 * s.scale) as i32;
                    
                    zoom_lbl.set_text(&format!("{:.0}%", s.scale * 100.0));
                    res_lbl.set_text(&format!("{}×{}", s.original_width, s.original_height));
                    drop(s);
                    
                    // 调整窗口大小
                    if let (Some(win), Some(da_inner)) = (&*win_load.borrow(), &*da_load.borrow()) {
                        update_window_size(win, da_inner, scaled_w, scaled_h);
                    }
                    
                    // 清除缓存
                    *cs.borrow_mut() = None;
                    cr_rot.set(-1);
                    da.queue_draw();
                    path_lbl.set_text(path);
                    path_lbl.set_tooltip_text(Some(path));
                }
                Err(e) => eprintln!("加载失败: {}", e),
            }
        })
    };

    // 初始加载图片，如果是 overlay 模式则在加载后启动
    if let Some(path) = initial_path {
        let load = load_image.clone();
        let start_overlay = initial_mode == WindowMode::Overlay;
        let app_init = app.clone();
        let state_init = state.clone();
        let overlay_pos_init = overlay_pos.clone();
        let overlay_window_init = overlay_window.clone();
        let current_mode_init = current_mode.clone();
        let window_ref_init = window_ref.clone();
        let da_ref_init = da_ref.clone();
        let window_init = window.clone();
        
        glib::idle_add_local_once(move || {
            load(&path);
            
            // 如果是 overlay 模式启动
            if start_overlay && state_init.borrow().pixbuf.is_some() {
                current_mode_init.set(WindowMode::Overlay);
                window_init.set_visible(false);
                
                // 计算居中位置
                let (scaled_w, scaled_h) = {
                    let s = state_init.borrow();
                    get_scaled_size(&s)
                };
                let (screen_w, screen_h) = get_screen_size();
                {
                    let mut pos = overlay_pos_init.borrow_mut();
                    pos.margin_left = (screen_w - scaled_w) / 2;
                    pos.margin_top = (screen_h - scaled_h) / 2;
                }
                
                let mode_exit = current_mode_init.clone();
                let win_ref_exit = window_ref_init.clone();
                let overlay_win_exit = overlay_window_init.clone();
                let state_exit = state_init.clone();
                let da_ref_exit = da_ref_init.clone();
                
                let overlay = create_overlay_window(
                    &app_init,
                    state_init.clone(),
                    overlay_pos_init.clone(),
                    move || {
                        mode_exit.set(WindowMode::Normal);
                        {
                            let mut s = state_exit.borrow_mut();
                            s.offset_x = 0.0;
                            s.offset_y = 0.0;
                        }
                        if let Some(ref win) = *win_ref_exit.borrow() {
                            win.set_visible(true);
                            win.present();
                            if let Some(ref da) = *da_ref_exit.borrow() {
                                da.queue_draw();
                            }
                        }
                        *overlay_win_exit.borrow_mut() = None;
                    },
                );
                overlay.present();
                *overlay_window_init.borrow_mut() = Some(overlay);
            }
        });
    }

    let win_open = window.clone();
    let load_open = load_image.clone();
    open_btn.connect_clicked(move |_| {
        let dialog = FileDialog::builder().title("选择图片").modal(true).build();
        let filter = gtk4::FileFilter::new();
        filter.add_mime_type("image/*");
        filter.set_name(Some("图片"));
        let filters = gio::ListStore::new::<gtk4::FileFilter>();
        filters.append(&filter);
        dialog.set_filters(Some(&filters));
        let load = load_open.clone();
        dialog.open(Some(&win_open), gio::Cancellable::NONE, move |r| {
            if let Ok(f) = r { if let Some(p) = f.path() { load(&p.to_string_lossy()); } }
        });
    });

    // 恢复视图
    let state_reset = state.clone();
    let da_reset = drawing_area.clone();
    let zoom_reset = zoom_label.clone();
    let win_reset = window_ref.clone();
    let da_reset_ref = da_ref.clone();
    reset_btn.connect_clicked(move |_| {
        let mut s = state_reset.borrow_mut();
        if s.pixbuf.is_some() {
            let (img_w, img_h) = match s.rotation % 2 {
                0 => (s.original_width, s.original_height),
                _ => (s.original_height, s.original_width),
            };
            let (target_w, target_h) = calc_target_size(img_w, img_h);
            let content_h = target_h - TITLEBAR_HEIGHT;
            s.scale = (target_w as f64 / img_w as f64).min(content_h as f64 / img_h as f64).min(1.0);
            s.offset_x = 0.0;
            s.offset_y = 0.0;
            
            let scaled_w = (img_w as f64 * s.scale) as i32;
            let scaled_h = (img_h as f64 * s.scale) as i32;
            zoom_reset.set_text(&format!("{:.0}%", s.scale * 100.0));
            drop(s);
            
            if let (Some(win), Some(da)) = (&*win_reset.borrow(), &*da_reset_ref.borrow()) {
                update_window_size(win, da, scaled_w, scaled_h);
            }
            da_reset.queue_draw();
        }
    });

    // 旋转
    let state_rotate = state.clone();
    let da_rotate = drawing_area.clone();
    rotate_btn.connect_clicked(move |_| {
        let mut s = state_rotate.borrow_mut();
        if s.pixbuf.is_some() {
            s.rotation = (s.rotation + 1) % 4;
            drop(s);
            da_rotate.queue_draw();
        }
    });

    // 复制到剪贴板
    let state_copy = state.clone();
    let win_copy = window.clone();
    copy_btn.connect_clicked(move |_| {
        let s = state_copy.borrow();
        if let Some(ref texture) = s.pixbuf {
            let clipboard = win_copy.clipboard();
            let content = gdk::ContentProvider::for_value(&texture.to_value());
            clipboard.set_content(Some(&content)).ok();
        }
    });

    // overlay 模式时先不显示普通窗口，等图片加载后直接显示 overlay
    if initial_mode != WindowMode::Overlay {
        window.present();
    }
}
