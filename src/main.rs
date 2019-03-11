
extern crate gimli;
extern crate object;
extern crate memmap;
extern crate fallible_iterator;


use object::Object;
use object::ObjectSection;
use fallible_iterator::FallibleIterator;


fn load_section<'a>(file: &object::File<'a>, section_name: &str) -> std::borrow::Cow<'a, [u8]> {
    return file.section_by_name(section_name).unwrap().data();
}


fn main() {
    let file_path = std::env::args().nth(1).expect("Missing argument");

    let file = match std::fs::File::open(&file_path) {
        Ok(file) => file,
        Err(err) => {
            println!("Error opening file '{}': {}", &file_path, err);
            return;
        }
    };

    let mmapped_file = match unsafe { memmap::Mmap::map(&file) } {
        Ok(mmapped_file) => mmapped_file,
        Err(err) => {
            println!("Could not map file '{}': {}", &file_path, err);
            return;
        }
    };

    let parsed_file = match object::File::parse(&*mmapped_file) {
        Ok(parsed_file) => parsed_file,
        Err(err) => {
            println!("Error parsing file '{}': {}", &file_path, err);
            return;
        }
    };

    let s_debug_info = load_section(&parsed_file, ".debug_info");
    let s_debug_abbrev = load_section(&parsed_file, ".debug_abbrev");

    // gimli stuff
    let debug_info = gimli::DebugInfo::new(&s_debug_info, gimli::LittleEndian);
    let debug_abbrev = gimli::DebugAbbrev::new(&s_debug_abbrev, gimli::LittleEndian);
    let dwarf = gimli::Dwarf {
        debug_info,
        debug_abbrev,
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
