// This code is from fontdb and keeps the same license, it is modified to support missing post script names

use cosmic::iced::advanced::graphics::text::cosmic_text::fontdb::{
    FaceInfo, ID, Source, Stretch, Style, Weight,
};
use ttf_parser::Language;

/// A list of possible font loading errors.
#[derive(Debug)]
pub enum LoadError {
    /// A malformed font.
    ///
    /// Typically means that [ttf-parser](https://github.com/RazrFalcon/ttf-parser)
    /// wasn't able to parse it.
    MalformedFont,
    /// A valid TrueType font without a valid *Family Name*.
    UnnamedFont,
    /// A file IO related error.
    IoError(std::io::Error),
}

impl From<std::io::Error> for LoadError {
    #[inline]
    fn from(e: std::io::Error) -> Self {
        LoadError::IoError(e)
    }
}

impl core::fmt::Display for LoadError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            LoadError::MalformedFont => write!(f, "malformed font"),
            LoadError::UnnamedFont => write!(f, "font doesn't have a family name"),
            LoadError::IoError(ref e) => write!(f, "{}", e),
        }
    }
}

pub fn parse_face_info<F: FnOnce() -> Option<(Vec<(String, Language)>, String)>>(
    source: Source,
    data: &[u8],
    index: u32,
    fallback_families: F,
) -> Result<FaceInfo, LoadError> {
    let raw_face = ttf_parser::RawFace::parse(data, index).map_err(|_| LoadError::MalformedFont)?;
    let (families, post_script_name) = parse_names(&raw_face)
        .or_else(fallback_families)
        .ok_or(LoadError::UnnamedFont)?;
    let (mut style, weight, stretch) = parse_os2(&raw_face);
    let (monospaced, italic) = parse_post(&raw_face);

    if style == Style::Normal && italic {
        style = Style::Italic;
    }

    Ok(FaceInfo {
        id: ID::dummy(),
        source,
        index,
        families,
        post_script_name,
        style,
        weight,
        stretch,
        monospaced,
    })
}

fn parse_names(raw_face: &ttf_parser::RawFace) -> Option<(Vec<(String, Language)>, String)> {
    const NAME_TAG: ttf_parser::Tag = ttf_parser::Tag::from_bytes(b"name");
    let name_data = raw_face.table(NAME_TAG)?;
    let name_table = ttf_parser::name::Table::parse(name_data)?;

    let mut families = collect_families(ttf_parser::name_id::TYPOGRAPHIC_FAMILY, &name_table.names);

    // We have to fallback to Family Name when no Typographic Family Name was set.
    if families.is_empty() {
        families = collect_families(ttf_parser::name_id::FAMILY, &name_table.names);
    }

    // Make English US the first one.
    if families.len() > 1 {
        if let Some(index) = families
            .iter()
            .position(|f| f.1 == Language::English_UnitedStates)
        {
            if index != 0 {
                families.swap(0, index);
            }
        }
    }

    if families.is_empty() {
        return None;
    }

    let post_script_name = name_table
        .names
        .into_iter()
        .find(|name| {
            name.name_id == ttf_parser::name_id::POST_SCRIPT_NAME && name.is_supported_encoding()
        })
        .and_then(|name| name_to_unicode(&name))?;

    Some((families, post_script_name))
}

fn collect_families(name_id: u16, names: &ttf_parser::name::Names) -> Vec<(String, Language)> {
    let mut families = Vec::new();
    for name in names.into_iter() {
        if name.name_id == name_id && name.is_unicode() {
            if let Some(family) = name_to_unicode(&name) {
                families.push((family, name.language()));
            }
        }
    }

    // If no Unicode English US family name was found then look for English MacRoman as well.
    if !families
        .iter()
        .any(|f| f.1 == Language::English_UnitedStates)
    {
        for name in names.into_iter() {
            if name.name_id == name_id && name.is_mac_roman() {
                if let Some(family) = name_to_unicode(&name) {
                    families.push((family, name.language()));
                    break;
                }
            }
        }
    }

    families
}

fn name_to_unicode(name: &ttf_parser::name::Name) -> Option<String> {
    if name.is_unicode() {
        let mut raw_data: Vec<u16> = Vec::new();
        for c in ttf_parser::LazyArray16::<u16>::new(name.name) {
            raw_data.push(c);
        }

        String::from_utf16(&raw_data).ok()
    } else if name.is_mac_roman() {
        // We support only MacRoman encoding here, which should be enough in most cases.
        let mut raw_data = Vec::with_capacity(name.name.len());
        for b in name.name {
            raw_data.push(MAC_ROMAN[*b as usize]);
        }

        String::from_utf16(&raw_data).ok()
    } else {
        None
    }
}

fn parse_os2(raw_face: &ttf_parser::RawFace) -> (Style, Weight, Stretch) {
    const OS2_TAG: ttf_parser::Tag = ttf_parser::Tag::from_bytes(b"OS/2");
    let table = match raw_face
        .table(OS2_TAG)
        .and_then(ttf_parser::os2::Table::parse)
    {
        Some(table) => table,
        None => return (Style::Normal, Weight::NORMAL, Stretch::Normal),
    };

    let style = match table.style() {
        ttf_parser::Style::Normal => Style::Normal,
        ttf_parser::Style::Italic => Style::Italic,
        ttf_parser::Style::Oblique => Style::Oblique,
    };

    let weight = table.weight();
    let stretch = table.width();

    (style, Weight(weight.to_number()), stretch)
}

fn parse_post(raw_face: &ttf_parser::RawFace) -> (bool, bool) {
    // We need just a single value from the `post` table, while ttf-parser will parse all.
    // Therefore we have a custom parser.

    const POST_TAG: ttf_parser::Tag = ttf_parser::Tag::from_bytes(b"post");
    let data = match raw_face.table(POST_TAG) {
        Some(v) => v,
        None => return (false, false),
    };

    // All we care about, it that u32 at offset 12 is non-zero.
    let monospaced = data.get(12..16) != Some(&[0, 0, 0, 0]);

    // Italic angle as f16.16.
    let italic = data.get(4..8) != Some(&[0, 0, 0, 0]);

    (monospaced, italic)
}

trait NameExt {
    fn is_mac_roman(&self) -> bool;
    fn is_supported_encoding(&self) -> bool;
}

impl NameExt for ttf_parser::name::Name<'_> {
    #[inline]
    fn is_mac_roman(&self) -> bool {
        use ttf_parser::PlatformId::Macintosh;
        // https://docs.microsoft.com/en-us/typography/opentype/spec/name#macintosh-encoding-ids-script-manager-codes
        const MACINTOSH_ROMAN_ENCODING_ID: u16 = 0;

        self.platform_id == Macintosh && self.encoding_id == MACINTOSH_ROMAN_ENCODING_ID
    }

    #[inline]
    fn is_supported_encoding(&self) -> bool {
        self.is_unicode() || self.is_mac_roman()
    }
}

/// Macintosh Roman to UTF-16 encoding table.
///
/// https://en.wikipedia.org/wiki/Mac_OS_Roman
#[rustfmt::skip]
const MAC_ROMAN: &[u16; 256] = &[
    0x0000, 0x0001, 0x0002, 0x0003, 0x0004, 0x0005, 0x0006, 0x0007,
    0x0008, 0x0009, 0x000A, 0x000B, 0x000C, 0x000D, 0x000E, 0x000F,
    0x0010, 0x2318, 0x21E7, 0x2325, 0x2303, 0x0015, 0x0016, 0x0017,
    0x0018, 0x0019, 0x001A, 0x001B, 0x001C, 0x001D, 0x001E, 0x001F,
    0x0020, 0x0021, 0x0022, 0x0023, 0x0024, 0x0025, 0x0026, 0x0027,
    0x0028, 0x0029, 0x002A, 0x002B, 0x002C, 0x002D, 0x002E, 0x002F,
    0x0030, 0x0031, 0x0032, 0x0033, 0x0034, 0x0035, 0x0036, 0x0037,
    0x0038, 0x0039, 0x003A, 0x003B, 0x003C, 0x003D, 0x003E, 0x003F,
    0x0040, 0x0041, 0x0042, 0x0043, 0x0044, 0x0045, 0x0046, 0x0047,
    0x0048, 0x0049, 0x004A, 0x004B, 0x004C, 0x004D, 0x004E, 0x004F,
    0x0050, 0x0051, 0x0052, 0x0053, 0x0054, 0x0055, 0x0056, 0x0057,
    0x0058, 0x0059, 0x005A, 0x005B, 0x005C, 0x005D, 0x005E, 0x005F,
    0x0060, 0x0061, 0x0062, 0x0063, 0x0064, 0x0065, 0x0066, 0x0067,
    0x0068, 0x0069, 0x006A, 0x006B, 0x006C, 0x006D, 0x006E, 0x006F,
    0x0070, 0x0071, 0x0072, 0x0073, 0x0074, 0x0075, 0x0076, 0x0077,
    0x0078, 0x0079, 0x007A, 0x007B, 0x007C, 0x007D, 0x007E, 0x007F,
    0x00C4, 0x00C5, 0x00C7, 0x00C9, 0x00D1, 0x00D6, 0x00DC, 0x00E1,
    0x00E0, 0x00E2, 0x00E4, 0x00E3, 0x00E5, 0x00E7, 0x00E9, 0x00E8,
    0x00EA, 0x00EB, 0x00ED, 0x00EC, 0x00EE, 0x00EF, 0x00F1, 0x00F3,
    0x00F2, 0x00F4, 0x00F6, 0x00F5, 0x00FA, 0x00F9, 0x00FB, 0x00FC,
    0x2020, 0x00B0, 0x00A2, 0x00A3, 0x00A7, 0x2022, 0x00B6, 0x00DF,
    0x00AE, 0x00A9, 0x2122, 0x00B4, 0x00A8, 0x2260, 0x00C6, 0x00D8,
    0x221E, 0x00B1, 0x2264, 0x2265, 0x00A5, 0x00B5, 0x2202, 0x2211,
    0x220F, 0x03C0, 0x222B, 0x00AA, 0x00BA, 0x03A9, 0x00E6, 0x00F8,
    0x00BF, 0x00A1, 0x00AC, 0x221A, 0x0192, 0x2248, 0x2206, 0x00AB,
    0x00BB, 0x2026, 0x00A0, 0x00C0, 0x00C3, 0x00D5, 0x0152, 0x0153,
    0x2013, 0x2014, 0x201C, 0x201D, 0x2018, 0x2019, 0x00F7, 0x25CA,
    0x00FF, 0x0178, 0x2044, 0x20AC, 0x2039, 0x203A, 0xFB01, 0xFB02,
    0x2021, 0x00B7, 0x201A, 0x201E, 0x2030, 0x00C2, 0x00CA, 0x00C1,
    0x00CB, 0x00C8, 0x00CD, 0x00CE, 0x00CF, 0x00CC, 0x00D3, 0x00D4,
    0xF8FF, 0x00D2, 0x00DA, 0x00DB, 0x00D9, 0x0131, 0x02C6, 0x02DC,
    0x00AF, 0x02D8, 0x02D9, 0x02DA, 0x00B8, 0x02DD, 0x02DB, 0x02C7,
];
