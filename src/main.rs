
extern crate gimli;
extern crate fallible_iterator;
extern crate goblin;


use fallible_iterator::FallibleIterator;
use std::io::Read;
use std::collections::HashMap;


fn main() {
    // open file
    let file_path = std::env::args().nth(1).expect("Missing argument");
    let mut file = match std::fs::File::open(&file_path) {
        Ok(file) => file,
        Err(err) => {
            println!("Error opening file '{}': {}", file_path, err);
            return;
        }
    };

    // parse Mach-O
    // TODO: support ELF, other formats
    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer).unwrap();
    let data = goblin::mach::MachO::parse(&buffer, 0).unwrap();

    // read dwarf sections
    let mut dwarf_sections: HashMap<String, &[u8]> = HashMap::new();
    for segment_sections in data.segments.sections() {
        for section in segment_sections {
            let (s, s_data) = section.unwrap();
            let s_segname = std::str::from_utf8(&s.segname)
                .unwrap()
                .to_string();
            let s_sectname = std::str::from_utf8(&s.sectname)
                .unwrap()
                .to_string();
            if s_segname.trim_matches(char::from(0)) == "__DWARF" {
                dwarf_sections.insert(s_sectname.trim_matches(char::from(0)).to_string(), &s_data);
            }
        }
    }

    macro_rules! load_section {
        ($x:ident, $s:expr) => (
            gimli::$x::new(
                dwarf_sections.get($s).expect("section not found"),
                gimli::LittleEndian
            );
        )
    }

    // parse dwarf sections with gimli
    let debug_info = load_section!(DebugInfo, "__debug_info");
    let debug_abbrev = load_section!(DebugAbbrev, "__debug_abbrev");
    let debug_line = load_section!(DebugLine, "__debug_line");
    let debug_str = load_section!(DebugStr, "__debug_str");
    let dwarf = gimli::Dwarf {
        debug_info,
        debug_abbrev,
        debug_line,
        debug_str,
        ..Default::default()
    };

    let units: Vec<_> = dwarf.units().collect().unwrap();
    for header in units {
        let unit = match dwarf.unit(header) {
            Ok(r) => r,
            Err(err) => {
                println!("error contructing unit for header {:?}: {}", header, err);
                continue;
            }
        };

        let mut entries = unit.entries();
        while let Some((_, entry)) = entries.next_dfs().unwrap() {
            if entry.tag() == gimli::DW_TAG_variable {
                let attrs: Vec<_> = entry.attrs().collect().unwrap();
                for attr in attrs {
                    println!("attr: {:?}", attr);
                }
            }
        }
    }
}
