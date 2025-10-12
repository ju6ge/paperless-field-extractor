pub(crate) struct Config {
    pub(crate) processing_tag: String,
    pub(crate) processing_color: String,
    pub(crate) finished_tag: String,
    pub(crate) finished_color: String,
    pub(crate) tag_user_name: String
}

impl Config {
    pub fn new<S: ToString>(processing_tag: S, finished_tag: S, tag_user: S) -> Self {
        Self {
            processing_tag: processing_tag.to_string(),
            processing_color: "#ffe000".to_string(),
            finished_tag: finished_tag.to_string(),
            finished_color: "#40aebf".to_string(),
            tag_user_name: tag_user.to_string()
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self::new(
            "🧠 processing",
            "🏷️ finished",
            "judge"
        )
    }
}
