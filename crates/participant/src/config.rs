//! The page's embedded state.

pub use rafi_live::PageConfig;

pub fn load() -> Option<PageConfig> {
    let json = web_sys::window()?
        .document()?
        .get_element_by_id("rafi-config")?
        .text_content()?;
    serde_json::from_str(&json).ok()
}
