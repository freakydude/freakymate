//! Brings a TOML document someone may have edited by hand in line with freshly serialized values, changing only what
//! differs: comments, blank lines, order and the formatting of unchanged values stay.

use toml_edit::{ArrayOfTables, Item, Table, Value};

/// Makes `target` hold the values of `source`. Keys missing in `source` (for example an `Option` that became `None`)
/// are removed.
pub(crate) fn merge_table(target: &mut Table, source: &Table) {
    let gone: Vec<String> =
        target.iter().map(|(key, _)| key.to_owned()).filter(|key| !source.contains_key(key)).collect();
    for key in gone {
        target.remove(&key);
    }
    for (key, new) in source.iter() {
        match target.get_mut(key) {
            Some(old) => merge_item(old, new),
            None => {
                target.insert(key, new.clone());
            }
        }
    }
}

fn merge_item(old: &mut Item, new: &Item) {
    match (old, new) {
        (Item::Table(old), Item::Table(new)) => merge_table(old, new),
        (Item::ArrayOfTables(old), Item::ArrayOfTables(new)) if old.len() == new.len() => merge_tables(old, new),
        (Item::Value(old), Item::Value(new)) => merge_value(old, new),
        (old, new) => *old = new.clone(),
    }
}

/// Table by table, so comments inside the blocks of a `[[list]]` stay as long as the number of entries stays.
fn merge_tables(old: &mut ArrayOfTables, new: &ArrayOfTables) {
    for (old, new) in old.iter_mut().zip(new.iter()) {
        merge_table(old, new);
    }
}

/// A changed value is replaced; the comment after it on the same line stays.
fn merge_value(old: &mut Value, new: &Value) {
    if bare(old) == bare(new) {
        return;
    }
    let decor = old.decor().clone();
    *old = new.clone();
    *old.decor_mut() = decor;
}

/// The value without the spaces and comments around it, to compare.
fn bare(value: &Value) -> String {
    let mut value = value.clone();
    value.decor_mut().clear();
    value.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use toml_edit::DocumentMut;

    fn merged(edited: &str, fresh: &str) -> String {
        let mut document: DocumentMut = edited.parse().unwrap();
        let fresh: DocumentMut = fresh.parse().unwrap();
        merge_table(document.as_table_mut(), fresh.as_table());
        document.to_string()
    }

    #[test]
    fn comments_and_unchanged_values_stay() {
        let edited = "# my printer\nfullscreen = true # kiosk\n\n[steps]\n# coarse\nmove_mm = 10.0\n";
        let fresh = "fullscreen = false\n\n[steps]\nmove_mm = 10.0\n";

        assert_eq!(
            merged(edited, fresh),
            "# my printer\nfullscreen = false # kiosk\n\n[steps]\n# coarse\nmove_mm = 10.0\n"
        );
    }

    #[test]
    fn comments_inside_a_list_of_tables_stay_while_its_length_stays() {
        let edited = "[[presets]]\n# for the satin one\nname = \"PLA\"\nbed = 60.0\n";
        let fresh = "[[presets]]\nname = \"PLA\"\nbed = 65.0\n";

        assert_eq!(merged(edited, fresh), "[[presets]]\n# for the satin one\nname = \"PLA\"\nbed = 65.0\n");
    }

    #[test]
    fn missing_keys_are_removed_and_new_ones_added() {
        let edited = "brightness = 40\nfullscreen = true\n";
        let fresh = "fullscreen = true\nverbose_log = false\n";

        assert_eq!(merged(edited, fresh), "fullscreen = true\nverbose_log = false\n");
    }
}
