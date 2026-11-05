// helterm patch: line break opportunities inside ligatures must not split the
// text into separately shaped words (adapted from cosmic-text 0.19).
use cosmic_text::{fontdb, Attrs, AttrsList, FontSystem, ShapeLine, Shaping};

fn words(font_system: &mut FontSystem, text: &str) -> usize {
    let line = ShapeLine::new(font_system, text, &AttrsList::new(&Attrs::new()), Shaping::Advanced, 4);
    line.spans[0].words.len()
}

#[test]
fn ligatures_stay_in_one_word() {
    let mut font_system = FontSystem::new_with_locale_and_db("en-US".into(), fontdb::Database::new());
    let font = std::fs::read("fonts/Inter-Regular.ttf").unwrap();
    font_system.db_mut().load_font_data(font);

    // Inter has contextual alternates for these; each has a break opportunity inside.
    assert_eq!(words(&mut font_system, "->"), 1);
    assert_eq!(words(&mut font_system, "|>"), 1);
    assert_eq!(words(&mut font_system, "!="), 1);
    // No ligature for "++": the break opportunity is kept.
    assert_eq!(words(&mut font_system, "++"), 2);
}
