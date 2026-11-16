fn main() {
    // Compiling the directory loads ui/slint-project.json: it names the main file and the Slint edition, which the
    // previews (scripts/ui-preview.sh) and the editor read from the same file.
    slint_build::compile("ui").expect("Slint UI must compile");
}
