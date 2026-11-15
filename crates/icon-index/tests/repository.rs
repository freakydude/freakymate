//! Checks the real icon set in the repository; fails if a file violates the rules
//! or `crates/ui/ui/icons.slint` does not match the folder contents.

use icon_index::{Layout, check_icons, load_icons, repository_root, slint_index};

#[test]
fn repository_icon_set_is_valid_and_index_is_current() {
    let layout = Layout::of_repository(&repository_root());
    let icons = load_icons(&layout.icons).expect("icon folder readable");

    assert!(!icons.is_empty());
    assert_eq!(check_icons(&icons), Vec::<String>::new());
    let current = std::fs::read_to_string(&layout.slint_index).expect("icons.slint exists").replace("\r\n", "\n");
    assert!(current == slint_index(&icons), "icons.slint is outdated, run: cargo run -p icon-index");
}
