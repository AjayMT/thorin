
extern crate gimli;
extern crate fallible_iterator;
extern crate goblin;


use fallible_iterator::FallibleIterator;
use std::io::Read;
use std::collections::HashMap;


struct Variable<'a> {
    name: &'a str,
    offset: i64
}


struct Scope<'a> {
    name: Option<&'a str>,
    variables: Vec<Variable<'a>>,
    scopes: Vec<Scope<'a>>
}


fn construct_global_scope<'a>(dwarf: &'a gimli::Dwarf<gimli::EndianSlice<gimli::LittleEndian>>) -> Scope<'a> {
    let mut scope = Scope { name: None, variables: Vec::new(), scopes: Vec::new() };

    let units: Vec<_> = dwarf.units().collect().unwrap();
    for header in units {
        let unit = match dwarf.unit(header) {
            Ok(r) => r,
            Err(err) => {
                println!("error constructing unit for header {:?}: {}", header, err);
                continue;
            }
        };

        let mut entries = unit.entries();
        while let Some((d_depth, entry)) = entries.next_dfs().unwrap() {
            if entry.tag() == gimli::DW_TAG_variable {
                let attrs: Vec<_> = entry.attrs().collect().unwrap();
                let mut name: Option<&str> = None;
                let mut offset: Option<i64> = None;
                for attr in attrs {
                    let attr_name = attr.name().static_string().unwrap();
                    let attr_value = attr.value();
                    match attr_name {
                        "DW_AT_name" => {
                            name = Some(dwarf.attr_string(&unit, attr_value).unwrap().to_string().unwrap())
                        },
                        "DW_AT_location" => {
                            let data = match attr_value {
                                gimli::AttributeValue::Exprloc(r) => r,
                                _ => { continue; }
                            };
                            let mut eval = data.evaluation(unit.encoding());
                            let mut eval_state = eval.evaluate().unwrap();
                            while eval_state != gimli::EvaluationResult::Complete {
                                match eval_state {
                                    gimli::EvaluationResult::RequiresFrameBase => {
                                        eval_state = eval.resume_with_frame_base(0).unwrap();
                                    },
                                    _ => unimplemented!()
                                }
                            }
                            let eval_result = eval.result();
                            if let gimli::Location::Address { address: addr } = eval_result[0].location {
                                offset = Some(addr as i64)
                            }
                        },
                        _ => { continue; }
                    }
                }

                if name.is_some() && offset.is_some() {
                    scope.variables.push(Variable { name: name.unwrap(), offset: offset.unwrap() });
                }
            }
        }
    }

    return scope;
}


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

    let global_scope = construct_global_scope(&dwarf);

    println!("global scope:");
    for var in global_scope.variables {
        println!("{}: fbreg {}", var.name, var.offset);
    }
}
