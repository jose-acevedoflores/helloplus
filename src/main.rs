#[macro_use]
extern crate conrod;
use api::{Api, SetData};
use conrod::backend::glium::glium::backend::glutin::glutin::VirtualKeyCode;
use conrod::backend::glium::glium::{self, Surface};
use conrod::glium::glutin::EventsLoop;
use conrod::glium::Display;
use conrod::image::Id;
use conrod::image::Map;
use conrod::{widget, Colorable, Positionable, Sizeable, Ui, UiCell, Widget};
use find_folder;
use image::imageops::FilterType;
use image::DynamicImage;
use log::{debug, info, warn};
use std::ops::Range;
use std::time::Instant;

const WIDTH: u32 = 1920;
const HEIGHT: u32 = 1080;

const NAVIGATION_KEYS_DEBOUNCE_THRESHOLD: u128 = 150;

const NUM_ROWS: usize = 4;
const ROW_STRIDE: usize = 6;
const NUM_IMAGES: usize = NUM_ROWS * ROW_STRIDE;

struct LeftRight(usize);
struct TopDown(usize);

type IsHighlighted = bool;

widget_ids!(struct Ids { titles[], imgs[], canvas[], img_not_found});

pub struct EventLoop {
    last_update: std::time::Instant,
}

impl EventLoop {
    pub fn new() -> Self {
        EventLoop {
            last_update: std::time::Instant::now(),
        }
    }

    /// Produce an iterator yielding all available events.
    pub fn next(
        &mut self,
        events_loop: &mut glium::glutin::EventsLoop,
    ) -> Vec<glium::glutin::Event> {
        // We don't want to loop any faster than 60 FPS, so wait until it has been at least 16ms
        // since the last yield.
        let last_update = self.last_update;
        let sixteen_ms = std::time::Duration::from_millis(16);
        let duration_since_last_update = std::time::Instant::now().duration_since(last_update);
        if duration_since_last_update < sixteen_ms {
            std::thread::sleep(sixteen_ms - duration_since_last_update);
        }

        // Collect all pending events.
        let mut events = Vec::new();
        events_loop.poll_events(|event| events.push(event));

        if events.is_empty() {
            events_loop.run_forever(|event| {
                events.push(event);
                glium::glutin::ControlFlow::Break
            });
        }

        self.last_update = std::time::Instant::now();

        events
    }
}

fn load_img(display: &glium::Display, dyn_img: DynamicImage) -> glium::texture::Texture2d {
    let rgba_image = dyn_img.to_rgba8();
    let image_dimensions = rgba_image.dimensions();
    let raw_image = glium::texture::RawImage2d::from_raw_rgba_reversed(
        &rgba_image.into_raw(),
        image_dimensions,
    );
    let texture = glium::texture::Texture2d::new(display, raw_image).unwrap();
    texture
}

fn load_img_not_found() -> DynamicImage {
    let assets = find_folder::Search::ParentsThenKids(3, 3)
        .for_folder("assets")
        .unwrap();
    let path = assets.join("images/image-not-found.png");
    let img = image::open(&std::path::Path::new(&path)).unwrap();
    img.resize(500, 220, FilterType::Lanczos3)
}

fn build_display() -> (Display, EventsLoop, Ui) {
    let events_loop = glium::glutin::EventsLoop::new();
    let window = glium::glutin::WindowBuilder::new()
        .with_title("Hello Plus")
        .with_dimensions(WIDTH, HEIGHT);
    let context = glium::glutin::ContextBuilder::new()
        .with_vsync(true)
        .with_multisampling(4);

    let mut ui = conrod::UiBuilder::new([WIDTH as f64, HEIGHT as f64]).build();
    load_fonts(&mut ui);
    (
        glium::Display::new(window, context, &events_loop).unwrap(),
        events_loop,
        ui,
    )
}

fn load_fonts(ui: &mut Ui) {
    let assets = find_folder::Search::KidsThenParents(3, 5)
        .for_folder("assets")
        .unwrap();
    let font_path = assets.join("fonts/NotoSans/NotoSans-Regular.ttf");
    ui.fonts.insert_from_file(font_path).unwrap();
}

struct SetRow<'a> {
    title: &'a str,
    set_data: SetData<'a>,
    set_id: usize,
    cached_img_id: Vec<(Id, f64, f64)>,
    rem: usize,
}

impl<'a> SetRow<'a> {
    fn new(set_data: SetData<'a>, set_idx: usize) -> Self {
        debug!("Initialized Set row: {:?}", set_data);
        let title = set_data.get_title();
        Self {
            set_data,
            title,
            set_id: set_idx,
            cached_img_id: Vec::new(),
            rem: (ROW_STRIDE * (set_idx + 1)),
        }
    }

    fn get_top_offset(&self, canvas_index: usize) -> f64 {
        (canvas_index as f64) * 290.0 + 70.0
    }

    fn get_left_offset(&self, item_idx: usize) -> f64 {
        (item_idx as f64) * 515.0 * 0.75 + 20.0
    }

    fn get_img_idx(&self, item_idx: usize, canvas_idx: usize) -> usize {
        (canvas_idx * ROW_STRIDE + item_idx) % self.rem
    }

    /// Sets the widget for a single image to be displayed for this row
    /// Returns true if this image should be highlighted (scaled up)
    ///
    /// NOTE: The reason we don't set the scaled up widget here is because when the image scales up
    /// it takes some space from the previous and next image. If we set the scale here then the next image
    /// will overlap and it will appear on top of the currently highlighted image. The scaled up
    /// image is drawn last to make sure it will be on top.
    fn show(
        &mut self,
        display: &Display,
        ui: &mut UiCell,
        image_map: &mut Map<glium::texture::Texture2d>,
        ids: &Ids,
        item_idx: usize,
        cursor: &Cursor,
        nf_id: &Id,
        canvas_idx: usize,
    ) -> IsHighlighted {
        let id = self.cached_img_id.get(item_idx);

        let (img_id, w, h) = if let Some((id, w, h)) = id {
            (id.clone(), w.clone(), h.clone())
        } else {
            let img = self.set_data.get_home_tile_image(item_idx);

            if let Ok(img) = img {
                let img = load_img(display, img);
                let (w, h) = (img.get_width(), img.get_height().unwrap());
                let img_id = image_map.insert(img);
                let w = (w as f64) * 0.75;
                let h = (h as f64) * 0.75;
                info!("put img {:?} ar {}", img_id, w / h);
                self.cached_img_id.push((img_id.clone(), w, h));
                (img_id, w, h)
            } else {
                self.cached_img_id
                    .push((nf_id.clone(), 500.0 * 0.75, 220.0 * 0.75));
                (nf_id.clone(), 500.0 * 0.75, 220.0 * 0.75)
            }
        };

        widget::Image::new(img_id)
            .w_h(w, h)
            .top_left_with_margins_on(
                ui.window,
                self.get_top_offset(canvas_idx),
                self.get_left_offset(item_idx),
            )
            .set(ids.imgs[self.get_img_idx(item_idx, canvas_idx)], ui);

        cursor.set_idx == self.set_id && cursor.item_idx == item_idx
    }

    /// Sets the text widget for the set title.
    fn show_row_title(&self, canvas_idx: usize, ids: &Ids, ui: &mut UiCell) {
        widget::Text::new(self.title)
            .up_from(ids.imgs[ROW_STRIDE * canvas_idx], 14.0)
            .color(conrod::color::WHITE)
            .font_size(28)
            .set(ids.titles[self.set_id % NUM_ROWS], ui);
    }
}

struct DisplayController<'a> {
    rows: Vec<SetRow<'a>>,
    display: &'a Display,
    image_map: Map<glium::texture::Texture2d>,
    api_handle: &'a Api,
    ids: Ids,
    nf_id: Id,
    prev_visible_range: Range<usize>,
}

impl<'a> DisplayController<'a> {
    fn new(display: &'a Display, api_handle: &'a Api, ui: &mut Ui) -> Self {
        let mut ids = Ids::new(ui.widget_id_generator());
        ids.imgs.resize(NUM_IMAGES, &mut ui.widget_id_generator());
        ids.titles.resize(NUM_ROWS, &mut ui.widget_id_generator());
        ids.canvas.resize(NUM_ROWS, &mut ui.widget_id_generator());

        let mut image_map = Map::<glium::texture::Texture2d>::new();
        let nf = load_img_not_found();
        let img = load_img(display, nf);
        let nf_id = image_map.insert(img);

        Self {
            rows: Vec::new(),
            display,
            image_map,
            api_handle,
            ids,
            nf_id,
            prev_visible_range: 0..NUM_ROWS,
        }
    }

    fn initialize(&mut self, ui: &mut Ui, cursor: &Cursor) {
        let ui = &mut ui.set_widgets();
        for canvas_idx in 0..NUM_ROWS {
            let row_data = self.api_handle.get_set(canvas_idx).expect("TODO testing");
            let mut set_row = SetRow::new(row_data, canvas_idx);
            for item_idx in 0..ROW_STRIDE {
                set_row.show(
                    self.display,
                    ui,
                    &mut self.image_map,
                    &self.ids,
                    item_idx,
                    &cursor,
                    &self.nf_id,
                    canvas_idx,
                );
            }
            set_row.show_row_title(canvas_idx, &self.ids, ui);
            self.rows.push(set_row);
        }
    }

    /// This function takes the set_idx and produces the range of sets that are going to be visible
    /// taking into account the expected number of visible rows.
    ///
    /// For example:
    ///  - with NUM_ROWS set to 4
    ///  - if set set_idx 0 through 2 the visible range is 0 to 4
    ///  - if user goes down 3 times now set_idx is 3 and visible range is 1 to 5
    ///  - if from 3 it goes to 4 then visible range now is 2 to 6
    ///  - if user now goes BACK so set_idx is back to 3 the range is still 2 to 6
    ///    This helps ease the transition since it won't jump all the rows back
    fn visible_range(&mut self, set_idx: usize) -> Range<usize> {
        if (set_idx - self.prev_visible_range.start) == 1 {
            return self.prev_visible_range.clone();
        }
        let new_range = if set_idx + 2 > NUM_ROWS {
            let shift = (set_idx + 2) - NUM_ROWS;
            shift..(shift + NUM_ROWS)
        } else {
            0..NUM_ROWS
        };

        self.prev_visible_range = new_range.clone();
        new_range
    }

    fn fetch_row(&mut self, set_idx: usize) {
        let res = self.rows.get_mut(set_idx);
        if res.is_none() {
            let row_data = self.api_handle.get_set(set_idx).expect("TODO testing");
            let set_row = SetRow::new(row_data, set_idx);
            self.rows.push(set_row);
            // self.rows.last().unwrap();
        }
    }

    fn navigate_to(&mut self, set_idx: usize, item_idx: usize, ui: &mut Ui) {
        info!("Image map size {}. idx:{}", self.image_map.len(), item_idx);
        let cursor = Cursor { set_idx, item_idx };
        let ui = &mut ui.set_widgets();
        let mut highlighted_idx = (0, 0, 0);
        for (canvas_idx, set_idx) in self.visible_range(set_idx).enumerate() {
            self.fetch_row(set_idx);
            let set_row = &mut self.rows[set_idx];
            for item_idx in 0..ROW_STRIDE {
                let found_highlighted = set_row.show(
                    self.display,
                    ui,
                    &mut self.image_map,
                    &self.ids,
                    item_idx,
                    &cursor,
                    &self.nf_id,
                    canvas_idx,
                );
                if found_highlighted {
                    highlighted_idx = (set_idx, item_idx, canvas_idx)
                }
            }
            set_row.show_row_title(canvas_idx, &self.ids, ui);
        }

        let (set_idx, item_idx, canvas_idx) = highlighted_idx;
        let row = &self.rows[set_idx];
        // //TODO FIX THIS
        let (img_id, w, h) = &row.cached_img_id[item_idx];
        widget::Image::new(img_id.clone())
            .w_h(w * 1.15, h * 1.15)
            .top_left_with_margins_on(
                ui.window,
                row.get_top_offset(canvas_idx) - 20.0,
                row.get_left_offset(item_idx) - 20.0,
            )
            .set(self.ids.imgs[row.get_img_idx(item_idx, canvas_idx)], ui);
    }
}

#[derive(Default)]
struct Cursor {
    set_idx: usize,
    item_idx: usize,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();
    let (display, mut events_loop, mut ui) = build_display();

    let api_handle = {
        let mut a = api::Api::new();
        a.load_home_data()?;
        a
    };

    let mut renderer = conrod::backend::glium::Renderer::new(&display).unwrap();
    let mut event_loop = EventLoop::new();

    let mut controller = DisplayController::new(&display, &api_handle, &mut ui);
    controller.initialize(&mut ui, &Cursor::default());

    let mut item_idx = LeftRight(0);
    let mut set_idx = TopDown(0);
    let mut navigation_debounce = Instant::now();

    'main: loop {
        // Render the `Ui` and then display it on the screen.
        if let Some(primitives) = ui.draw_if_changed() {
            renderer.fill(&display, primitives, &controller.image_map);
            let mut target = display.draw();
            target.clear_color(0.0, 0.0, 0.013, 1.0);
            renderer
                .draw(&display, &mut target, &controller.image_map)
                .unwrap();
            target.finish().unwrap();
        }
        let mut events = Vec::new();
        events_loop.poll_events(|event| events.push(event));

        for event in event_loop.next(&mut events_loop) {
            match event {
                glium::glutin::Event::WindowEvent { event, .. } => match event {
                    glium::glutin::WindowEvent::Closed
                    | glium::glutin::WindowEvent::KeyboardInput {
                        input:
                            glium::glutin::KeyboardInput {
                                virtual_keycode: Some(glium::glutin::VirtualKeyCode::Escape),
                                ..
                            },
                        ..
                    } => break 'main,
                    glium::glutin::WindowEvent::KeyboardInput {
                        input:
                            glium::glutin::KeyboardInput {
                                virtual_keycode: Some(key_code),
                                ..
                            },
                        ..
                    } => {
                        if user_navigated(
                            key_code,
                            &mut item_idx,
                            &mut set_idx,
                            &mut navigation_debounce,
                        ) {
                            controller.navigate_to(set_idx.0, item_idx.0, &mut ui);
                        } else {
                            println!("event {:?}", key_code);
                        }
                    }
                    _ => (),
                },
                _ => (),
            }
        }
    }
    Ok(())
}

fn user_navigated(
    key_code: VirtualKeyCode,
    item: &mut LeftRight,
    set_num: &mut TopDown,
    navigation_debounce: &mut Instant,
) -> bool {
    if (*navigation_debounce).elapsed().as_millis() < NAVIGATION_KEYS_DEBOUNCE_THRESHOLD {
        return false;
    }
    *navigation_debounce = Instant::now();
    if key_code == VirtualKeyCode::Left {
        if (*item).0 != 0 {
            (*item).0 -= 1;
        }
        return true;
    } else if key_code == VirtualKeyCode::Right {
        (*item).0 += 1;
        return true;
    } else if key_code == VirtualKeyCode::Up {
        if (*set_num).0 != 0 {
            (*set_num).0 -= 1;
        }
        return true;
    } else if key_code == VirtualKeyCode::Down {
        (*set_num).0 += 1;
        return true;
    }

    return false;
}
