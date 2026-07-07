//! Preset glyph ramps, ordered dark -> light.

/// Look up a preset ramp by name, or return `name` itself as a custom ramp.
pub fn resolve(name: &str) -> String {
    match name {
        "standard" => " .:-=+*#%@",
        "blocks" => " ░▒▓█",
        "simple" => " .*#",
        "binary" => " .#",
        "shaded" => " ▏▎▍▌▋▊▉█",
        "density" => " .,:;i1tfLCG08@",
        "portrait" => " .,:;irsXA253hMHGS#9B&@",
        "dots" => " ....::::;;;;iiii111222333555999###@@",
        // Classic 70-level Paul Bourke ramp — most tonal detail.
        "detailed" => r#" .'`^",:;Il!i><~+_-?][}{1)(|\/tfjrxnuvczXYUJCLQ0OZmwqpdbkhao*#MW&8%B@$"#,
        "moon" => "🌑🌒🌓🌔🌕", // emoji brightness ramp
        _ => name, // custom ramp string
    }
    .to_string()
}

pub const PRESETS: &[&str] = &[
    "standard", "blocks", "simple", "binary", "shaded", "density", "portrait", "dots", "detailed",
    "moon",
];
