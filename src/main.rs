
extern crate gimli;
extern crate fallible_iterator;
extern crate goblin;


use fallible_iterator::FallibleIterator;
use std::io::Read;
use std::collections::HashMap;


fn main() {
    let file_path = std::env::args().nth(1).expect("Missing argument");
    let mut file = match std::fs::File::open(&file_path) {
        Ok(file) => file,
        Err(err) => {
            println!("Error opening file '{}': {}", &file_path, err);
            return;
        }
    };

    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer).unwrap();
    let data = goblin::mach::MachO::parse(&buffer, 0).unwrap();

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

    let s_debug_info = dwarf_sections.get("__debug_info").expect("No debug_info found");
    let s_debug_abbrev = dwarf_sections.get("__debug_abbrev").expect("No debug_abbrev found");
    let s_debug_line = dwarf_sections.get("__debug_line").expect("No debug_line found");

    // gimli stuff
    let debug_info = gimli::DebugInfo::new(&s_debug_info, gimli::LittleEndian);
    let debug_abbrev = gimli::DebugAbbrev::new(&s_debug_abbrev, gimli::LittleEndian);
    let debug_line = gimli::DebugLine::new(&s_debug_line, gimli::LittleEndian);
    let dwarf = gimli::Dwarf {
        debug_info,
        debug_abbrev,
        debug_line,
        ..Default::default()
    };

    let units = dwarf.units().collect::<Vec<_>>().unwrap();
    for header in units {
        let unit = match dwarf.unit(header) {
            Ok(r) => r,
            Err(err) => {
                println!("error contructing unit for header {:?}: {}", header, err);
                continue;
            }
        };

        println!("unit name {:?}", unit.name.unwrap());
    }
}
