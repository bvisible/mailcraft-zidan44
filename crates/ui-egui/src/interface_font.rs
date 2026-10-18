// //// Neoffice — added file (no upstream equivalent): the interface's own font in the browser, Karla regular and bold
// (the Neoffice desk's font). The desktop app is given the system's font and its bold (`install_fonts`); the browser
// has no system font to give, and without one the bold family fell back to the regular font: the B of the editor,
// the unread mail of the list and every heading showed nothing bold (maintenance#1673).

/// A font the interface is drawn with, regular and bold, with the licence that goes with it.
pub struct InterfaceFont {
    pub family: &'static str,
    /// The licence's identifier and its text, shown on the About page.
    pub licence: &'static str,
    pub licence_text: &'static str,
    pub regular: &'static [u8],
    pub bold: &'static [u8],
}

/// Karla 2.004 (googlefonts/karla), the static Latin instances: about 40 KB each.
pub const KARLA: InterfaceFont = InterfaceFont {
    family: "Karla",
    licence: "OFL-1.1",
    licence_text: include_str!("../fonts/karla/OFL.txt"),
    regular: include_bytes!("../fonts/karla/Karla-Regular.ttf"),
    bold: include_bytes!("../fonts/karla/Karla-Bold.ttf"),
};

impl InterfaceFont {
    /// The licence as its authors wrote it: without the line that marks its file as the fork's.
    pub fn licence(&self) -> String {
        self.licence_text.lines().filter(|l| !l.starts_with("# //// Neoffice")).collect::<Vec<_>>().join("\n")
    }
}

/// The font this build draws with when it is given none: Karla in the browser, nothing on the desktop, where the
/// system's font is drawn with (and egui's own, when the system has none).
pub fn built_in() -> Option<&'static InterfaceFont> {
    if cfg!(target_arch = "wasm32") { Some(&KARLA) } else { None }
}

/// The fonts to install: those given (the system's), or else the built-in pair.
pub fn or_built_in(regular: Option<Vec<u8>>, bold: Option<Vec<u8>>, built_in: Option<&InterfaceFont>) -> (Option<Vec<u8>>, Option<Vec<u8>>) {
    match (regular, bold, built_in) {
        (None, None, Some(f)) => (Some(f.regular.to_vec()), Some(f.bold.to_vec())),
        (regular, bold, _) => (regular, bold),
    }
}
