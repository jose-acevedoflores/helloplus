//! ## Hello+
//!
//! Clone of a streaming service homepage using [`conrod glium/winit`](https://github.com/PistonDevelopers/conrod)
#[macro_use]
extern crate conrod;
use api::{Api, SetData};
use conrod::backend::glium::glium::backend::glutin::glutin::VirtualKeyCode;
use conrod::backend::glium::glium::{self, Surface};
use conrod::glium::Display;
use conrod::image::Id;
use conrod::image::Map;
use conrod::{widget, Colorable, Positionable, Sizeable, Ui, UiCell, Widget};
use log::{debug, info};
use std::ops::Range;
use std::time::Instant;
mod helpers;

const DISPLAY_WIDTH: u32 = 1920;
const DISPLAY_HEIGHT: u32 = 1080;

/// Debounce value for handling the Left, Right, Up Down key strokes.
const NAVIGATION_KEYS_DEBOUNCE_THRESHOLD: u128 = 180;
/// This field represents the number of visible rows given the [`ROW_HEIGHT`],the [`ROW_TOP_MARGIN`] and the [`DISPLAY_HEIGHT`]
const NUM_ROWS: usize = 4;
/// This field serves as the number of spaces reserved in the [Ids::imgs] field for a given row.
/// This is adjusted to keep at least one out of view image in memory so the user doesn't see a placeholder.
const ROW_STRIDE: usize = 6;
/// This represents the number of images available to draw. Used for various alignments and as the total size of the [Ids::imgs] field.
const NUM_OF_CACHED_IMAGES: usize = NUM_ROWS * ROW_STRIDE;

// **** Start of pixel alignment consts.
/// Margin to space out the thumbnails. Used to the left and right of the images.
const ITEMS_MARGIN: f64 = 20.0;
const IMAGE_WIDTH_PLUS_MARGIN: f64 = 500.0 + 15.0;
const IMAGE_SCALE_DOWN_FACTOR: f64 = 0.75;
/// Used to scale the image up so that it looks 15% larger.
const IMAGE_SCALE_UP_FACTOR: f64 = 1.15;
const ROW_TOP_MARGIN: f64 = 70.0;
const ROW_HEIGHT: f64 = 290.0;

widget_ids!(
    /// Hold the [`Id`]s for the row titles and the images
    struct Ids {
        titles[],
        imgs[]
    }
);

/// In order to not spin endlessly this struct will throttle the main loop and queue incoming events.
/// It will throttle to target 60fps rate.
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

struct CachedImgData {
    img_id: Id,
    w: f64,
    h: f64,
}

impl CachedImgData {
    fn new(img_id: Id, w: f64, h: f64) -> Self {
        Self { img_id, w, h }
    }
}

/// Holds the necessary data needed to draw a single row.
///
/// Its responsibilities include:
/// - fetching the resources (title and thumbnail images) and caching them.
/// - place the visible widgets in the appropriate locations given the combination of `true` and `adjusted` indices.
/// - controls the navigation for this row via the [`shift_right`](SetRow::shift_right) and [`shift_left`](SetRow::shift_left) methods.
struct SetRow<'a> {
    /// Title for this set of data.
    title: &'a str,
    /// The fetched data retrieved by the [`Api`].
    set_data: SetData<'a>,
    /// Unique id for this set of data.
    true_set_idx: usize,
    /// Cached [`Id`] keys used to map the image data stored in the [`image_map`](DisplayController::image_map)
    /// IMPROVEMENT: treat as a fixed sized array to only keep the
    cached_img_id: Vec<CachedImgData>,
    /// Combined with the `adjusted_item_idx` it produces the `true_item_idx` for this specific row.
    left_right_idx_adjustment: usize,
}

impl<'a> SetRow<'a> {
    /// Constructor.
    fn new(set_data: SetData<'a>, true_set_idx: usize) -> Self {
        debug!("Initialized Set row: {:?}", set_data);
        let title = set_data.get_title();
        Self {
            set_data,
            title,
            true_set_idx,
            cached_img_id: Vec::new(),
            left_right_idx_adjustment: 0,
        }
    }

    /// Shift right on a given row. Returns a bool because it needs to check that row's specific
    /// item count.
    /// # Arguments
    /// * `adjusted_item_idx`: this is the index that stays between the 0 to [`ROW_STRIDE`]-1 range.
    /// * `true_item_idx`: this is the full index into this row's items.
    fn shift_right(&mut self, adjusted_item_idx: usize, true_item_idx: usize) -> bool {
        if (true_item_idx + 1) < self.set_data.get_item_count() {
            if adjusted_item_idx + 4 > ROW_STRIDE {
                self.left_right_idx_adjustment += 1;
            }
            true
        } else {
            false
        }
    }

    ///
    /// # Arguments
    /// * `adjusted_item_idx`: this is the index that stays between the 0 to [`ROW_STRIDE`]-1 range.
    fn shift_left(&mut self, adjusted_item_idx: usize) {
        if self.left_right_idx_adjustment > 0 {
            if adjusted_item_idx < 2 {
                self.left_right_idx_adjustment -= 1;
            }
        }
    }

    ///
    /// # Arguments
    /// * `adjusted_set_idx`: This is the canvas index for this set of data. This index is adjusted to
    ///    stay between 0 and [`NUM_ROWS`]-1
    fn get_top_offset(&self, adjusted_set_idx: usize) -> f64 {
        (adjusted_set_idx as f64) * ROW_HEIGHT + ROW_TOP_MARGIN
    }

    /// # Arguments
    /// * `adjusted_item_idx`: this is the canvas index for the item (always between 0 and [`ROW_STRIDE`]-1).
    fn get_left_offset(&self, adjusted_item_idx: usize) -> f64 {
        (adjusted_item_idx as f64) * IMAGE_WIDTH_PLUS_MARGIN * IMAGE_SCALE_DOWN_FACTOR
            + ITEMS_MARGIN
    }

    ///
    /// # Arguments
    /// * `adjusted_item_idx`: this is the canvas index for the item (always between 0 and [`ROW_STRIDE`]-1).
    /// * `adjusted_set_idx`: This is the canvas index for this set of data. This index is adjusted to
    ///    stay between 0 and [`NUM_ROWS`]-1
    fn get_img_idx(&self, adjusted_item_idx: usize, adjusted_set_idx: usize) -> usize {
        adjusted_set_idx * ROW_STRIDE + adjusted_item_idx
    }

    /// Sets the widget to display the appropriate image for this row given the `adjusted_*` indices.
    /// Returns true if this image should be highlighted (scaled up).
    ///
    /// NOTE: The reason we don't set the scaled up widget here is because when the image scales up
    /// it takes some space from the previous and next image. If we set the scale here then the next image
    /// will overlap and it will appear on top of the currently highlighted image. The scaled up
    /// image is drawn last to make sure it will be on top.
    /// # Argumets
    /// * `adjusted_item_idx`: this is the canvas index for the item (always between 0 and [`ROW_STRIDE`]-1).
    /// * `adjusted_set_idx`: This is the canvas index for this set of data. This index is adjusted to
    ///    stay between 0 and [`NUM_ROWS`]-1
    fn show(
        &mut self,
        display: &Display,
        ui: &mut UiCell,
        image_map: &mut Map<glium::texture::Texture2d>,
        ids: &Ids,
        cursor: &Cursor,
        nf_id: &Id,
        adjusted_item_idx: usize,
        adjusted_set_idx: usize,
    ) -> Option<HighlightedItemData> {
        let true_item_idx = adjusted_item_idx + self.left_right_idx_adjustment;

        if self.cached_img_id.get(true_item_idx).is_none() {
            let img = self.set_data.get_home_tile_image(true_item_idx);

            if let Ok(img) = img {
                let img = helpers::load_img(display, img);
                let (w, h) = (img.get_width(), img.get_height().unwrap());
                let img_id = image_map.insert(img);
                let w = (w as f64) * IMAGE_SCALE_DOWN_FACTOR;
                let h = (h as f64) * IMAGE_SCALE_DOWN_FACTOR;
                info!("put img {:?} ar {}", img_id, w / h);
                self.cached_img_id.push(CachedImgData::new(img_id, w, h));
            } else {
                self.cached_img_id.push(CachedImgData::new(
                    nf_id.clone(),
                    500.0 * 0.75,
                    220.0 * 0.75,
                ));
            }
        };

        // We know that from the previous if block there will be an item at true_item_idx now
        let data = self.cached_img_id.get(true_item_idx).unwrap();

        let hd =
            if cursor.true_set_idx == self.true_set_idx && cursor.true_item_idx == true_item_idx {
                Some(HighlightedItemData {
                    img_id: data.img_id,
                    w: data.w,
                    h: data.h,
                    true_set_idx: self.true_set_idx,
                    adjusted_item_idx,
                    adjusted_set_idx,
                })
            } else {
                None
            };

        self.draw_image(
            data.img_id,
            data.w,
            data.h,
            adjusted_set_idx,
            adjusted_item_idx,
            ids,
            ui,
        );

        // Return true if this item needs to be scaled up (highlighted)
        hd
    }

    fn draw_image(
        &self,
        img_id: Id,
        w: f64,
        h: f64,
        adjusted_set_idx: usize,
        adjusted_item_idx: usize,
        ids: &Ids,
        ui: &mut UiCell,
    ) {
        widget::Image::new(img_id)
            .w_h(w, h)
            .top_left_with_margins_on(
                ui.window,
                self.get_top_offset(adjusted_set_idx),
                self.get_left_offset(adjusted_item_idx),
            )
            .set(
                ids.imgs[self.get_img_idx(adjusted_item_idx, adjusted_set_idx)],
                ui,
            );
    }

    /// Enlarges the image by [`IMAGE_SCALE_UP_FACTOR`] and also moves it back and up by [`ITEMS_MARGIN`].
    fn draw_image_highlighted(
        &self,
        img_id: Id,
        w: f64,
        h: f64,
        adjusted_set_idx: usize,
        adjusted_item_idx: usize,
        ids: &Ids,
        ui: &mut UiCell,
    ) {
        widget::Image::new(img_id)
            .w_h(w * IMAGE_SCALE_UP_FACTOR, h * IMAGE_SCALE_UP_FACTOR)
            .top_left_with_margins_on(
                ui.window,
                self.get_top_offset(adjusted_set_idx) - ITEMS_MARGIN,
                self.get_left_offset(adjusted_item_idx) - ITEMS_MARGIN,
            )
            .set(
                ids.imgs[self.get_img_idx(adjusted_item_idx, adjusted_set_idx)],
                ui,
            );
    }

    /// Sets the text widget for the set title.
    ///
    /// This method places the index above the first leftmost image for a given set (`adjusted_set_idx`)
    /// # Arguments
    /// * `adjusted_set_idx`: This is the canvas index for this set of data. This index is adjusted to
    ///    stay between 0 and [`NUM_ROWS`]-1
    fn show_row_title(&self, adjusted_set_idx: usize, ids: &Ids, ui: &mut UiCell) {
        widget::Text::new(self.title)
            .up_from(ids.imgs[ROW_STRIDE * adjusted_set_idx], 24.0)
            .color(conrod::color::WHITE)
            .font_size(28)
            .set(ids.titles[self.true_set_idx % NUM_ROWS], ui);
    }
}

/// Main structure controlling the widgets that should be displayed.
/// Its main responsibility is interpreting the navigation commands (Left, Right, Up or Down)
/// and adjust the internal state to reflect what should be displayed.
struct DisplayController<'a> {
    initialized: bool,
    rows: Vec<SetRow<'a>>,
    display: &'a Display,
    image_map: Map<glium::texture::Texture2d>,
    api_handle: &'a Api,
    ids: Ids,
    nf_id: Id,
    prev_visible_range: Range<usize>,
    cursor: Cursor,
}

impl<'a> DisplayController<'a> {
    fn new(display: &'a Display, api_handle: &'a Api, ui: &mut Ui) -> Self {
        let mut ids = Ids::new(ui.widget_id_generator());
        ids.imgs
            .resize(NUM_OF_CACHED_IMAGES, &mut ui.widget_id_generator());
        ids.titles.resize(NUM_ROWS, &mut ui.widget_id_generator());

        let mut image_map = Map::<glium::texture::Texture2d>::new();
        let nf = helpers::load_img_not_found();
        let img = helpers::load_img(display, nf);
        let nf_id = image_map.insert(img);

        Self {
            initialized: false,
            rows: Vec::new(),
            display,
            image_map,
            api_handle,
            ids,
            nf_id,
            prev_visible_range: 0..NUM_ROWS,
            cursor: Cursor::default(),
        }
    }

    /// Initialize the DisplayController. This is meant to be called once at start of the program.
    fn initialize(&mut self, ui: &mut Ui, cursor: &Cursor) {
        if self.initialized {
            return;
        }
        self.initialized = true;
        //NOTE: in this method, `true` amd `adjusted` indices are the same.
        let ui = &mut ui.set_widgets();
        for set_idx in self.prev_visible_range.clone() {
            let row_data = self.api_handle.get_set(set_idx).expect("TODO testing");
            let mut set_row = SetRow::new(row_data, set_idx);
            for item_idx in 0..ROW_STRIDE {
                set_row.show(
                    self.display,
                    ui,
                    &mut self.image_map,
                    &self.ids,
                    &cursor,
                    &self.nf_id,
                    item_idx,
                    set_idx,
                );
            }
            set_row.show_row_title(set_idx, &self.ids, ui);
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
    fn visible_set_range(&mut self, set_idx: usize) -> Range<usize> {
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

    /// This associated function is meant to be the access point of the Self.rows vector.
    fn fetch_row<'b>(
        rows: &'b mut Vec<SetRow<'a>>,
        true_set_idx: usize,
        api_handle: &'a Api,
    ) -> Option<&'b mut SetRow<'a>> {
        if rows.get_mut(true_set_idx).is_some() {
            return rows.get_mut(true_set_idx);
        }
        // we know res is none so need to fetch the data for this set.
        let set_row_opt = if let Some(row_data) = api_handle.get_set(true_set_idx) {
            let set_row = SetRow::new(row_data, true_set_idx);
            Some(set_row)
        } else {
            None
        };
        if let Some(set_row) = set_row_opt {
            rows.push(set_row);
            rows.last_mut()
        } else {
            None
        }
    }

    fn update_image_widgets(&mut self, ui: &mut Ui) {
        info!(
            "Image map size {}. idx:{}",
            self.image_map.len(),
            self.cursor.true_item_idx
        );
        let ui = &mut ui.set_widgets();
        let mut highlighted_data = None;
        for (adjusted_set_idx, true_set_idx) in
            self.visible_set_range(self.cursor.true_set_idx).enumerate()
        {
            let fetched = Self::fetch_row(&mut self.rows, true_set_idx, self.api_handle);
            if fetched.is_none() {
                break;
            }
            let set_row = fetched.unwrap();
            for adjusted_item_idx in 0..ROW_STRIDE {
                let found_highlighted = set_row.show(
                    self.display,
                    ui,
                    &mut self.image_map,
                    &self.ids,
                    &self.cursor,
                    &self.nf_id,
                    adjusted_item_idx,
                    adjusted_set_idx,
                );
                if found_highlighted.is_some() {
                    highlighted_data = found_highlighted;
                }
            }
            set_row.show_row_title(adjusted_set_idx, &self.ids, ui);
        }

        if let Some(HighlightedItemData {
            img_id,
            w,
            h,
            true_set_idx,
            adjusted_item_idx,
            adjusted_set_idx,
        }) = highlighted_data
        {
            self.cursor.adjusted_item_idx = adjusted_item_idx;
            if let Some(highlighted_row) =
                Self::fetch_row(&mut self.rows, true_set_idx, self.api_handle)
            {
                highlighted_row.draw_image_highlighted(
                    img_id,
                    w,
                    h,
                    adjusted_set_idx,
                    adjusted_item_idx,
                    &self.ids,
                    ui,
                );
            }
        }
    }

    pub(crate) fn move_current_set_left(&mut self, ui: &mut Ui) {
        if let Some(cur_row_data) =
            Self::fetch_row(&mut self.rows, self.cursor.true_set_idx, self.api_handle)
        {
            cur_row_data.shift_left(self.cursor.adjusted_item_idx);
            if self.cursor.true_item_idx > 0 {
                self.cursor.true_item_idx -= 1;
            }
            self.update_image_widgets(ui);
        }
    }

    pub(crate) fn move_current_set_right(&mut self, ui: &mut Ui) {
        if let Some(cur_row_data) =
            Self::fetch_row(&mut self.rows, self.cursor.true_set_idx, self.api_handle)
        {
            if cur_row_data.shift_right(self.cursor.adjusted_item_idx, self.cursor.true_item_idx) {
                self.cursor.true_item_idx += 1;
            }
            self.update_image_widgets(ui);
        }
    }

    pub(crate) fn move_to_prev_set(&mut self, ui: &mut Ui) {
        if self.cursor.true_set_idx > 0 {
            self.cursor.true_set_idx -= 1;
            if let Some(cur_row_data) =
                Self::fetch_row(&mut self.rows, self.cursor.true_set_idx, self.api_handle)
            {
                self.cursor.true_item_idx =
                    self.cursor.adjusted_item_idx + cur_row_data.left_right_idx_adjustment;
            }
        }
        self.update_image_widgets(ui);
    }

    pub(crate) fn move_to_next_set(&mut self, ui: &mut Ui) {
        if self.cursor.true_set_idx < self.api_handle.get_num_of_sets().unwrap() - 1 {
            self.cursor.true_set_idx += 1;
            if let Some(cur_row_data) =
                Self::fetch_row(&mut self.rows, self.cursor.true_set_idx, self.api_handle)
            {
                self.cursor.true_item_idx =
                    self.cursor.adjusted_item_idx + cur_row_data.left_right_idx_adjustment;
            }
        }
        self.update_image_widgets(ui);
    }
}

/// Represents where the cursor is at on the screen. By cursor, it really means what are the indices
/// of the highlighted item.
#[derive(Default)]
struct Cursor {
    true_set_idx: usize,
    true_item_idx: usize,
    adjusted_item_idx: usize,
}

struct HighlightedItemData {
    img_id: Id,
    w: f64,
    h: f64,
    true_set_idx: usize,
    adjusted_item_idx: usize,
    adjusted_set_idx: usize,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();
    let (display, mut events_loop, mut ui) = helpers::build_display();

    let api_handle = {
        let mut a = api::Api::new();
        a.load_home_data()?;
        a
    };

    let mut renderer = conrod::backend::glium::Renderer::new(&display).unwrap();
    let mut event_loop = EventLoop::new();

    let mut controller = DisplayController::new(&display, &api_handle, &mut ui);
    controller.initialize(&mut ui, &Cursor::default());

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
                        if navigation_debounce.elapsed().as_millis()
                            < NAVIGATION_KEYS_DEBOUNCE_THRESHOLD
                        {
                            break;
                        }
                        navigation_debounce = Instant::now();

                        if key_code == VirtualKeyCode::Left {
                            controller.move_current_set_left(&mut ui);
                        } else if key_code == VirtualKeyCode::Right {
                            controller.move_current_set_right(&mut ui);
                        } else if key_code == VirtualKeyCode::Up {
                            controller.move_to_prev_set(&mut ui);
                        } else if key_code == VirtualKeyCode::Down {
                            controller.move_to_next_set(&mut ui);
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
