//! ## Api
//!
//! The main responsibility of the [`Api`] crate is to abstract away the interactions with the backend.
//! It's in charge of fetching and parsing the json data.
//!
//! ### Improvements
//! - It could shed unused fields to lower the memory footprint.
//! - It could provide some caching.
use image::io::Reader as ImageReader;
use image::{DynamicImage, ImageFormat};
use log::info;
use reqwest;
use reqwest::StatusCode;
use serde_json::Value;
use std::io::Cursor;

/// Struct used to interact with the backend.
pub struct Api {
    json_data: Option<Value>,
    //Consider a cached layer to avoid fetching resources already here.
}

const TITLE_NOT_FOUND: &str = "Title not found";
const TILE_TYPE_DEFAULT: &str = "program";

/// Struct that encapsulates a given set's data.
///
/// Provides methods to interact with the data like:
/// - getting the title
/// - getting how many items are in this set
/// - getting the image to display for a given item on this set
#[derive(Debug)]
pub struct SetData<'a> {
    entry: &'a Value,
}

impl<'a> SetData<'a> {
    fn new(entry: &'a Value) -> Self {
        Self { entry }
    }

    pub fn get_title(&self) -> &'a str {
        if let Value::String(ref s) =
            self.entry["text"]["title"]["full"]["set"]["default"]["content"]
        {
            s
        } else {
            TITLE_NOT_FOUND
        }
    }

    pub fn get_item_count(&self) -> usize {
        if let Value::Array(ref vec) = self.entry["items"] {
            vec.len()
        } else {
            0
        }
    }

    /// This method parses the set and fetches the url to be used for the tile.
    /// Assumes the following attribute path:
    ///
    /// > `.items[IDX].image.tile[AR].<series|program>.default.url`
    ///
    /// Where `IDX` is an index
    /// Where `AR` is the aspect ratio
    ///
    pub fn get_home_tile_image(
        &self,
        item_num: usize,
    ) -> Result<DynamicImage, Box<dyn std::error::Error>> {
        if let Value::Object(ref map) = self.entry["items"][item_num]["image"]["tile"] {
            let (key, tile_data) = map
                .iter()
                .reduce(|cur, prev| {
                    let cur_key = cur.0;
                    let cur_key = cur_key.parse::<f32>().expect("float value");

                    let prev_key = prev.0;
                    let prev_key = prev_key.parse::<f32>().expect("float value");

                    if cur_key > prev_key {
                        cur
                    } else {
                        prev
                    }
                })
                .expect("some tile data to be present"); //TODO improve this error handling

            let tile_type = if let Value::Object(ref map) = tile_data {
                map.keys().into_iter().last().unwrap().as_str()
            } else {
                TILE_TYPE_DEFAULT
            };

            if let Value::String(ref url) = tile_data[tile_type]["default"]["url"] {
                let response = reqwest::blocking::get(url)?;
                if response.status() != StatusCode::OK {
                    info!("Status not good for item {} and key {}", item_num, key);
                }
                let buf = response.bytes()?;
                let img = ImageReader::with_format(Cursor::new(buf), ImageFormat::Jpeg).decode()?;
                Ok(img)
            } else {
                let err_msg = format!("No url found for item num: '{}'", item_num);
                Err(err_msg.into())
            }
        } else {
            let err_msg = format!("Did not find tile image for item num: '{}'", item_num);
            Err(err_msg.into())
        }
    }
}

impl Api {
    /// New up an empty [`Api`]. To populate call load ['Api.load`]
    pub fn new() -> Self {
        Self { json_data: None }
    }

    pub fn load_home_data(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let resp =
            reqwest::blocking::get("https://cd-static.bamgrid.com/dp-117731241344/home.json")?
                .json::<Value>()?;
        self.json_data.replace(resp);

        Ok(())
    }

    /// Attempt to get the [`SetData`] for the given `set_idx`
    pub fn get_set(&self, set_idx: usize) -> Option<SetData> {
        if let Some(data) = self.json_data.as_ref() {
            let ct = &data["data"]["StandardCollection"]["containers"];

            if let Value::Array(arr) = ct {
                if set_idx >= arr.len() {
                    return None;
                }
            }

            let res = &ct[set_idx]["set"];
            let set = SetData::new(res);
            Some(set)
        } else {
            None
        }
    }

    /// Returns the number of containers that were previously loaded.
    /// Returns None if the api has not fetched any data.
    pub fn get_num_of_sets(&self) -> Option<usize> {
        if let Some(data) = self.json_data.as_ref() {
            let ct = &data["data"]["StandardCollection"]["containers"];

            if let Value::Array(arr) = ct {
                return Some(arr.len());
            }
        }
        None
    }
}
