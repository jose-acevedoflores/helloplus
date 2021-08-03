//! ## Hello+
//!
//! Clone of a streaming service homepage using [`conrod glium/winit`](https://github.com/PistonDevelopers/conrod)
//!
//! The basic idea is to load a simulated home page from some JSON data and populate the simulated
//! 1920x1080 window.
//!
//! The idea is to keep around as few resources as possible while still allowing fluid navigation.
//! To achieve this we are running vectors with fixed sizes to cache the minimum amount of data.
//!
//! It also uses an indexing scheme to allow the reuse slots inside the [Ids::imgs] and [Ids::titles] buffers.
//! It consists of:
//!  - having an `adjusted_set` index for the data set (corresponds to the rows) and an `adjusted_item` index
//!    for an item within a dataset (corresponds to a specific image). These `adjusted`
//!    indices never exceed a predetermined value. This value is calculated based on the size of the window
//!    and the size of the individual items to make sure whatever the user is currently pointing to
//!    is always in view.
//!
//! ### Improvements
//! - [`DisplayController image_map`](DisplayController::disp_ctrl_img_data) is currently "unbounded".
//!   It is technically bound by how many sets/items are fetched from the json data.
//! - [`SetRow::cached_img_id`] is also unbounded and could be set so that it follows the [Ids::imgs] pattern.
//! - Currently, at start time, everything is loaded in one shot. It would be better to
//!   break that out to work alongside the [`EventLoop`] to load rows dynamically to give the user some quick feedback.
//!
#![allow(rustdoc::private_intra_doc_links)]
#![warn(missing_docs)]

#[macro_use]
extern crate conrod;
use api::{Api, SetData};
use conrod::backend::glium::glium::backend::glutin::glutin::VirtualKeyCode;
use conrod::backend::glium::glium::{self, Surface};
use conrod::glium::Display;
use conrod::image::Id;
use conrod::image::Map;
use conrod::{widget, Colorable, Positionable, Sizeable, Ui, UiCell, Widget};
use log::{debug, info, trace};
use std::cell::RefCell;
use std::ops::{Deref, Range};
use std::rc::Rc;
use std::time::Instant;

mod helpers;

const DISPLAY_WIDTH: u32 = 1920;
const DISPLAY_HEIGHT: u32 = 1080;

/// Limit the time between fetches for the artwork.
/// This helps give some times for input events to flow through even when we are still loading images.
const ITEM_LOADING_LOOP_THRESHOLD: u128 = 190;
/// Debounce value for handling the Left, Right, Up Down key strokes.
const NAVIGATION_KEYS_DEBOUNCE_THRESHOLD: u128 = 180;
/// We don't want to loop any faster than 60 FPS, so wait until it has been at least 16ms
/// since the last yield.
const MAIN_LOOP_TIME_FREQUENCY: u64 = 16;

/// This field represents the number of visible rows given the [`ROW_HEIGHT`],the [`ROW_TOP_MARGIN`] and the [`DISPLAY_HEIGHT`]
const NUM_ROWS: usize = 4;
/// This field serves as the number of spaces reserved in the [Ids::imgs] field for a given row.
/// This is adjusted to keep at least one out of view image in memory so the user doesn't see a placeholder.
const ROW_STRIDE: usize = 6;
/// This represents the number of images available to draw. Used for various alignments and as the total size of the [Ids::imgs] field.
const NUM_OF_CACHED_IMAGES: usize = NUM_ROWS * ROW_STRIDE;
/// This field represents the number of ROWS kept in memory.
const BUFFERED_ROWS: usize = 6;
/// This field represents how many images are loaded on a single loop of the MAIN_LOOP.
/// Used to improve responsiveness.
const SINGLE_LOOP_MAX_LOAD: usize = 2;

// **** Start of pixel alignment consts.
/// Margin to space out the thumbnails. Used to the left and right of the images.
const ITEMS_MARGIN: f64 = 20.0;
const IMAGE_WIDTH_PLUS_MARGIN: f64 = 500.0 + 15.0;
const IMAGE_SCALE_DOWN_FACTOR: f64 = 0.75;
/// Used to scale the image up so that it looks 15% larger.
const IMAGE_SCALE_UP_FACTOR: f64 = 1.15;
const ROW_TOP_MARGIN: f64 = 70.0;
const ROW_HEIGHT: f64 = 290.0;

const PLACEHOLDER_AND_NOT_FOUND_SCALED_W: f64 = 500.0 * IMAGE_SCALE_DOWN_FACTOR;
const PLACEHOLDER_AND_NOT_FOUND_SCALED_H: f64 = 220.0 * IMAGE_SCALE_DOWN_FACTOR;

struct AdjustedIndices {
    adjusted_set_idx: usize,
    adjusted_item_idx: usize,
}

struct Dimensions {
    w: f64,
    h: f64,
}

widget_ids!(
    /// Hold the [`Id`]s for the row titles and the images.
    /// Note that `imgs` length is [`NUM_OF_CACHED_IMAGES`].
    ///
    /// The scheme used for the `imgs` field is that continuous chunks (sized [`ROW_STRIDE`]) of data are used
    /// to store the images in view.
    ///
    /// For example:
    ///  - With  [`NUM_ROWS`] set to 4 and [`ROW_STRIDE`] set to 6, `imgs` will have 24 elements.
    ///  - This produces an array that looks like:
    ///
    /// | 0, 1, 2, 3, 4, 5, | 6, 7, 8, 9, 10, 11,| 12, 13, 14, 15, 16, 17,| 18, 19, 20, 21, 22, 23 |
    /// | ----------------- | ------------------ | ---------------------- | ---------------------- |
    /// | indices for row 0 |  indices for row 1 |   indices for row 2    |   indices for row 3    |
    ///
    struct Ids {
        titles[],
        imgs[]
    }
);

/// In order to not spin endlessly this struct will throttle the main loop and queue incoming events.
/// It will throttle to target 60fps rate.
pub struct EventLoop {
    last_update: std::time::Instant,
    img_load_pending: Rc<ImgLoadingNotifier>,
}

impl EventLoop {
    /// Constructor.
    pub fn new(img_load_pending: Rc<ImgLoadingNotifier>) -> Self {
        Self {
            last_update: std::time::Instant::now(),
            img_load_pending,
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
        let sixteen_ms = std::time::Duration::from_millis(MAIN_LOOP_TIME_FREQUENCY);
        let duration_since_last_update = std::time::Instant::now().duration_since(last_update);
        if duration_since_last_update < sixteen_ms {
            std::thread::sleep(sixteen_ms - duration_since_last_update);
        }

        // Collect all pending events.
        let mut events = Vec::new();
        events_loop.poll_events(|event| events.push(event));
        if events.is_empty() && !*self.img_load_pending.needs_to_load.borrow() {
            debug!("parking until next event");
            events_loop.run_forever(|event| {
                events.push(event);
                glium::glutin::ControlFlow::Break
            });
        }

        self.last_update = std::time::Instant::now();

        events
    }
}

/// Simple holder to keep track of the img_ids we've already placed in the [`image_map`](DisplayController::disp_ctrl_img_data)
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
    /// Cached [`Id`] keys used to map the image data stored in the [`image_map`](DisplayController::disp_ctrl_img_data)
    ///
    /// IMPROVEMENT: treat as a fixed sized array to only keep the items in view.
    cached_img_id: Vec<CachedImgData>,
    /// Combined with the `adjusted_item_idx` it produces the `true_item_idx` for this specific row.
    left_right_idx_adjustment: usize,
}

impl<'a> SetRow<'a> {
    /// Constructor.
    fn new(set_data: SetData<'a>, true_set_idx: usize) -> Self {
        trace!("Initialized Set row: {:?}", set_data);
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
    /// * `adjusted_item_idx`: this is the canvas index for the item (always between 0 and [`ROW_STRIDE`]-1).
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
    /// * `adjusted_item_idx`: this is the canvas index for the item (always between 0 and [`ROW_STRIDE`]-1).
    fn shift_left(&mut self, adjusted_item_idx: usize) {
        if self.left_right_idx_adjustment > 0 && adjusted_item_idx < 2 {
            self.left_right_idx_adjustment -= 1;
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

    fn get_home_tile_or_not_found(
        &self,
        display: &Display,
        true_item_idx: usize,
        image_map: &mut Map<glium::texture::Texture2d>,
        nf_id: &Id,
    ) -> CachedImgData {
        let img = self.set_data.get_home_tile_image(true_item_idx);

        if let Ok(img) = img {
            let img = helpers::load_img(display, img);
            let (w, h) = (img.get_width(), img.get_height().unwrap());
            let img_id = image_map.insert(img);
            let w = (w as f64) * IMAGE_SCALE_DOWN_FACTOR;
            let h = (h as f64) * IMAGE_SCALE_DOWN_FACTOR;
            info!("put img {:?} ar {}", img_id, w / h);
            CachedImgData::new(img_id, w, h)
        } else {
            CachedImgData::new(
                *nf_id,
                PLACEHOLDER_AND_NOT_FOUND_SCALED_W,
                PLACEHOLDER_AND_NOT_FOUND_SCALED_H,
            )
        }
    }

    fn populate_cache_if_needed(
        &mut self,
        display: &Display,
        true_item_idx: usize,
        disp_ctrl_img_data: &mut DispCtrlImgData,
        img_load_pending: &ImgLoadingNotifier,
    ) {
        let image_map = &mut disp_ctrl_img_data.image_map;

        let is_cached_already = self.cached_img_id.get(true_item_idx).is_some();

        let can_load_more =
            img_load_pending.single_loop_load_count.borrow().deref() < &SINGLE_LOOP_MAX_LOAD;

        if is_cached_already {
            let is_placeholder = self
                .cached_img_id
                .get(true_item_idx)
                .as_ref()
                .unwrap()
                .img_id
                == disp_ctrl_img_data.placeholder_id;

            if is_placeholder && can_load_more {
                let cached_img = self.get_home_tile_or_not_found(
                    display,
                    true_item_idx,
                    image_map,
                    &disp_ctrl_img_data.nf_id,
                );

                // Replace the previously cached img.
                self.cached_img_id[true_item_idx] = cached_img;
                img_load_pending.image_loaded();
            } else if is_placeholder && !can_load_more {
                // There are placeholders still in view, need to tell main loop to pass again.
                *img_load_pending.needs_to_load.borrow_mut() = true;
            }
        } else if can_load_more {
            let cached_img = self.get_home_tile_or_not_found(
                display,
                true_item_idx,
                image_map,
                &disp_ctrl_img_data.nf_id,
            );

            // Add new image.
            self.cached_img_id.push(cached_img);
            img_load_pending.image_loaded();
        } else {
            self.cached_img_id.push(CachedImgData::new(
                disp_ctrl_img_data.placeholder_id,
                PLACEHOLDER_AND_NOT_FOUND_SCALED_W,
                PLACEHOLDER_AND_NOT_FOUND_SCALED_H,
            ));

            *img_load_pending.needs_to_load.borrow_mut() = true;
        }
    }

    /// Sets the widget to display the appropriate image for this row given the `adjusted_*` indices.
    /// Returns true if this image should be highlighted (scaled up).
    ///
    /// NOTE: The reason we don't set the scaled up widget here is because when the image scales up,
    /// it takes some space from the previous and next image. If we set the scaled up image here then the next image
    /// will overlap and it will appear on top of the currently highlighted image. The scaled up
    /// image is drawn last to make sure it will be on top.
    /// # Arguments
    /// * `adjusted_item_idx`: this is the canvas index for the item (always between 0 and [`ROW_STRIDE`]-1).
    /// * `adjusted_set_idx`: This is the canvas index for this set of data. This index is adjusted to
    ///    stay between 0 and [`NUM_ROWS`]-1
    fn show(
        &mut self,
        display: &Display,
        ui: &mut UiCell,
        disp_ctrl_img_data: &mut DispCtrlImgData,
        cursor: &Cursor,
        adjusted_indices: AdjustedIndices,
        img_load_pending: &ImgLoadingNotifier,
    ) -> Option<HighlightedItemData> {
        let AdjustedIndices {
            adjusted_set_idx,
            adjusted_item_idx,
        } = adjusted_indices;

        let true_item_idx = adjusted_item_idx + self.left_right_idx_adjustment;

        self.populate_cache_if_needed(display, true_item_idx, disp_ctrl_img_data, img_load_pending);

        // We know that from the previous call to populate_cache_if_needed there will be an item at true_item_idx now
        let data = self.cached_img_id.get(true_item_idx).unwrap();

        let ids = &disp_ctrl_img_data.ids;

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

        let adjusted_indices = AdjustedIndices {
            adjusted_set_idx,
            adjusted_item_idx,
        };
        self.draw_image(
            data.img_id,
            Dimensions {
                w: data.w,
                h: data.h,
            },
            adjusted_indices,
            ids,
            ui,
        );

        // Return 'Some' if this item needs to be scaled up (highlighted)
        hd
    }

    fn draw_image(
        &self,
        img_id: Id,
        dims: Dimensions,
        adjusted_indices: AdjustedIndices,
        ids: &Ids,
        ui: &mut UiCell,
    ) {
        let Dimensions { w, h } = dims;
        let AdjustedIndices {
            adjusted_set_idx,
            adjusted_item_idx,
        } = adjusted_indices;

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
        dims: Dimensions,
        adjusted_indices: AdjustedIndices,
        ids: &Ids,
        ui: &mut UiCell,
    ) {
        let Dimensions { w, h } = dims;
        let AdjustedIndices {
            adjusted_set_idx,
            adjusted_item_idx,
        } = adjusted_indices;

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

/// Group the Image Id related fields
struct DispCtrlImgData {
    ids: Ids,
    nf_id: Id,
    placeholder_id: Id,
    image_map: Map<glium::texture::Texture2d>,
}

/// Main structure controlling the widgets that should be displayed.
/// Its main responsibility is interpreting the navigation commands (Left, Right, Up or Down)
/// and adjust the internal state to reflect what should be displayed.
struct DisplayController<'a> {
    initialized: bool,
    rows: Vec<SetRow<'a>>,
    display: &'a Display,
    disp_ctrl_img_data: DispCtrlImgData,
    api_handle: &'a Api,
    prev_visible_range: Range<usize>,
    cursor: Cursor,
    img_load_pending: &'a ImgLoadingNotifier,
}

impl<'a> DisplayController<'a> {
    fn new(
        display: &'a Display,
        api_handle: &'a Api,
        ui: &mut Ui,
        img_load_pending: &'a ImgLoadingNotifier,
    ) -> Self {
        let mut ids = Ids::new(ui.widget_id_generator());
        ids.imgs
            .resize(NUM_OF_CACHED_IMAGES, &mut ui.widget_id_generator());
        ids.titles.resize(NUM_ROWS, &mut ui.widget_id_generator());

        let mut image_map = Map::<glium::texture::Texture2d>::new();
        let nf = helpers::load_img_not_found();
        let img = helpers::load_img(display, nf);
        let nf_id = image_map.insert(img);

        let placeholder = helpers::load_placeholder_img();
        let placeholder_img = helpers::load_img(display, placeholder);
        let placeholder_id = image_map.insert(placeholder_img);

        let disp_ctrl_img_data = DispCtrlImgData {
            ids,
            nf_id,
            placeholder_id,
            image_map,
        };

        Self {
            initialized: false,
            rows: Vec::with_capacity(BUFFERED_ROWS),
            display,
            disp_ctrl_img_data,
            api_handle,
            prev_visible_range: 0..NUM_ROWS,
            cursor: Cursor::default(),
            img_load_pending,
        }
    }

    /// Initialize the [`DisplayController`]. This is meant to be called once at start of the program.
    fn initialize(&mut self, ui: &mut Ui, cursor: &Cursor) {
        if self.initialized {
            return;
        }
        self.initialized = true;
        //NOTE: in this method, `true` amd `adjusted` indices are the same.
        let ui = &mut ui.set_widgets();
        for set_idx in self.prev_visible_range.clone() {
            let row_data = self.api_handle.get_set(set_idx).unwrap();
            let mut set_row = SetRow::new(row_data, set_idx);
            for item_idx in 0..ROW_STRIDE {
                let adjusted_indices = AdjustedIndices {
                    adjusted_set_idx: set_idx,
                    adjusted_item_idx: item_idx,
                };
                set_row.show(
                    self.display,
                    ui,
                    &mut self.disp_ctrl_img_data,
                    &cursor,
                    adjusted_indices,
                    self.img_load_pending,
                );
            }
            set_row.show_row_title(set_idx, &self.disp_ctrl_img_data.ids, ui);
            self.rows.push(set_row);
        }
    }

    /// This function takes the `true_set_index` and produces the range of sets that are going to be visible
    /// taking into account the expected number of visible rows.
    ///
    /// For example:
    ///  - with [`NUM_ROWS`] set to 4
    ///  - if set set_idx 0 through 2 the visible range is 0 to 4
    ///  - if user goes down 3 times now set_idx is 3 and visible range is 1 to 5
    ///  - if from 3 it goes to 4 then visible range now is 2 to 6
    ///  - if user now goes BACK so set_idx is back to 3 the range is still 2 to 6
    ///    This helps ease the transition since it won't jump all the rows back
    fn visible_set_range(&mut self, true_set_index: usize) -> Range<usize> {
        if (true_set_index - self.prev_visible_range.start) == 1 {
            return self.prev_visible_range.clone();
        }
        let new_range = if true_set_index + 2 > NUM_ROWS {
            let shift = (true_set_index + 2) - NUM_ROWS;
            shift..(shift + NUM_ROWS)
        } else {
            0..NUM_ROWS
        };

        self.prev_visible_range = new_range.clone();
        new_range
    }

    /// This associated function is meant to be the access point of the [`rows`](DisplayController::rows) vector.
    fn fetch_row<'b>(
        rows: &'b mut Vec<SetRow<'a>>,
        true_set_idx: usize,
        api_handle: &'a Api,
    ) -> Option<&'b mut SetRow<'a>> {
        if rows.get_mut(true_set_idx % BUFFERED_ROWS).is_some()
            && rows
                .get_mut(true_set_idx % BUFFERED_ROWS)
                .unwrap()
                .true_set_idx
                == true_set_idx
        {
            return rows.get_mut(true_set_idx % BUFFERED_ROWS);
        }
        // we know res is none so need to fetch the data for this set.
        let set_row_opt = if let Some(row_data) = api_handle.get_set(true_set_idx) {
            let set_row = SetRow::new(row_data, true_set_idx);
            Some(set_row)
        } else {
            None
        };
        if let Some(set_row) = set_row_opt {
            if true_set_idx % BUFFERED_ROWS >= rows.len() {
                rows.push(set_row);
            } else {
                rows[true_set_idx % BUFFERED_ROWS] = set_row;
            }
            rows.get_mut(true_set_idx % BUFFERED_ROWS)
        } else {
            None
        }
    }

    fn update_image_widgets(&mut self, ui: &mut Ui) {
        info!(
            "Image map size {}. idx:{}",
            self.disp_ctrl_img_data.image_map.len(),
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
                let adjusted_indices = AdjustedIndices {
                    adjusted_set_idx,
                    adjusted_item_idx,
                };
                let found_highlighted = set_row.show(
                    self.display,
                    ui,
                    &mut self.disp_ctrl_img_data,
                    &self.cursor,
                    adjusted_indices,
                    self.img_load_pending,
                );
                if found_highlighted.is_some() {
                    highlighted_data = found_highlighted;
                }
            }
            set_row.show_row_title(adjusted_set_idx, &self.disp_ctrl_img_data.ids, ui);
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
                let dim = Dimensions { w, h };
                let idx = AdjustedIndices {
                    adjusted_set_idx,
                    adjusted_item_idx,
                };
                highlighted_row.draw_image_highlighted(
                    img_id,
                    dim,
                    idx,
                    &self.disp_ctrl_img_data.ids,
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

/// Encapsulates the data of the item that should be highlighted so that it can be drawn last.
struct HighlightedItemData {
    img_id: Id,
    w: f64,
    h: f64,
    true_set_idx: usize,
    adjusted_item_idx: usize,
    adjusted_set_idx: usize,
}

/// Struct to communicate to the [`EventLoop`] that there is still data to be loaded.
pub struct ImgLoadingNotifier {
    needs_to_load: RefCell<bool>,
    single_loop_load_count: RefCell<usize>,
    last_download_time: RefCell<Option<Instant>>,
}

impl ImgLoadingNotifier {
    fn reset(&self) {
        if let Some(last_update) = *self.last_download_time.borrow() {
            let dur = std::time::Instant::now().duration_since(last_update);
            if dur.as_millis() < ITEM_LOADING_LOOP_THRESHOLD {
                return;
            }
        }
        *self.single_loop_load_count.borrow_mut() = 0;
        *self.needs_to_load.borrow_mut() = false;
        *self.last_download_time.borrow_mut() = None;
    }

    fn image_loaded(&self) {
        *self.single_loop_load_count.borrow_mut() += 1;
        *self.last_download_time.borrow_mut() = Some(Instant::now());
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::builder().format_timestamp_millis().init();
    let (display, mut events_loop, mut ui) = helpers::build_display();

    let api_handle = {
        let mut a = api::Api::new();
        a.load_home_data()?;
        a
    };
    let img_load_pending = Rc::new(ImgLoadingNotifier {
        needs_to_load: RefCell::new(true),
        single_loop_load_count: RefCell::new(0),
        last_download_time: RefCell::new(None),
    });

    let mut renderer = conrod::backend::glium::Renderer::new(&display).unwrap();

    let mut controller = DisplayController::new(&display, &api_handle, &mut ui, &img_load_pending);
    controller.initialize(&mut ui, &Cursor::default());

    let mut event_loop = EventLoop::new(Rc::clone(&img_load_pending));

    let mut navigation_debounce = Instant::now();

    'main: loop {
        debug!("Main loop top");
        // Render the `Ui` and then display it on the screen.
        if let Some(primitives) = ui.draw_if_changed() {
            debug!("ui needs redraw");
            renderer.fill(
                &display,
                primitives,
                &controller.disp_ctrl_img_data.image_map,
            );
            let mut target = display.draw();
            target.clear_color(0.0, 0.0, 0.013, 1.0);
            renderer
                .draw(
                    &display,
                    &mut target,
                    &controller.disp_ctrl_img_data.image_map,
                )
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
        if *img_load_pending.needs_to_load.borrow() {

            img_load_pending.reset();
            controller.update_image_widgets(&mut ui);
        }
    }
    Ok(())
}
