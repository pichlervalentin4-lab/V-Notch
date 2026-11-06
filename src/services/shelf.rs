//! The file shelf: files dropped on the notch wait here until they are dragged out again.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ShelfFile {
    pub path: PathBuf,
    #[serde(skip)]
    pub selected: bool,
}

impl ShelfFile {
    pub fn name(&self) -> String {
        self.path.file_name().map_or_else(|| self.path.to_string_lossy().into_owned(), |n| n.to_string_lossy().into_owned())
    }

    pub fn is_image(&self) -> bool {
        self.path.extension().and_then(|e| e.to_str()).is_some_and(|ext| {
            matches!(ext.to_ascii_lowercase().as_str(), "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp")
        })
    }
}

#[derive(Clone, Debug, Default)]
pub struct Shelf {
    pub files: Vec<ShelfFile>,
    demo: bool,
}

impl Shelf {
    fn file() -> PathBuf {
        crate::paths::data_dir().join("shelf.json")
    }

    /// Files deleted while the app was closed are dropped from the list.
    pub fn load() -> Self {
        let files: Vec<ShelfFile> =
            std::fs::read_to_string(Self::file()).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default();
        let files = files.into_iter().filter(|f| f.path.exists()).collect();
        Self { files, demo: false }
    }

    fn save(&self) {
        if self.demo {
            return;
        }
        if let Ok(text) = serde_json::to_string(&self.files) {
            let _ = std::fs::write(Self::file(), text);
        }
    }

    pub fn add(&mut self, path: &Path) {
        if !self.files.iter().any(|f| f.path == path) {
            self.files.push(ShelfFile { path: path.to_path_buf(), selected: false });
            self.save();
        }
    }

    pub fn remove_selected(&mut self) {
        self.files.retain(|f| !f.selected);
        self.save();
    }

    pub fn remove(&mut self, path: &Path) {
        self.files.retain(|f| f.path != path);
        self.save();
    }

    pub fn clear(&mut self) {
        self.files.clear();
        self.save();
    }

    pub fn toggle(&mut self, index: usize) {
        if let Some(file) = self.files.get_mut(index) {
            file.selected = !file.selected;
        }
    }

    pub fn select_only(&mut self, index: usize) {
        for (i, file) in self.files.iter_mut().enumerate() {
            file.selected = i == index;
        }
    }

    pub fn select_all(&mut self, selected: bool) {
        for file in &mut self.files {
            file.selected = selected;
        }
    }

    /// The paths a drag takes along: the selection, or just the grabbed file if it is not part of it.
    pub fn drag_paths(&self, grabbed: usize) -> Vec<PathBuf> {
        let Some(file) = self.files.get(grabbed) else { return Vec::new() };
        if file.selected {
            self.files.iter().filter(|f| f.selected).map(|f| f.path.clone()).collect()
        } else {
            vec![file.path.clone()]
        }
    }

    pub fn demo_fill(&mut self) {
        self.demo = true;
        for name in ["Skript.pdf", "Foto.png", "Musik.mp3", "Notizen.txt", "Projekt.zip"] {
            self.files.push(ShelfFile { path: PathBuf::from(name), selected: false });
        }
    }

    pub fn selected_count(&self) -> usize {
        self.files.iter().filter(|f| f.selected).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shelf() -> Shelf {
        let mut shelf = Shelf::default();
        shelf.demo = true;
        shelf.add(Path::new("a.txt"));
        shelf.add(Path::new("b.png"));
        shelf.add(Path::new("c.pdf"));
        shelf
    }

    #[test]
    fn adding_the_same_file_twice_keeps_one() {
        let mut shelf = shelf();
        shelf.add(Path::new("a.txt"));
        assert_eq!(shelf.files.len(), 3);
    }

    #[test]
    fn drag_takes_the_selection_only_if_grabbed_is_part_of_it() {
        let mut shelf = shelf();
        shelf.toggle(0);
        shelf.toggle(1);
        assert_eq!(shelf.drag_paths(1).len(), 2);
        assert_eq!(shelf.drag_paths(2), vec![PathBuf::from("c.pdf")]);
        assert!(shelf.drag_paths(99).is_empty());
    }

    #[test]
    fn removing_selected_keeps_the_rest() {
        let mut shelf = shelf();
        shelf.select_only(1);
        shelf.remove_selected();
        assert_eq!(shelf.files.len(), 2);
        assert!(shelf.files.iter().all(|f| f.name() != "b.png"));
    }

    #[test]
    fn images_are_recognized() {
        assert!(ShelfFile { path: "x.PNG".into(), selected: false }.is_image());
        assert!(!ShelfFile { path: "x.pdf".into(), selected: false }.is_image());
    }
}
