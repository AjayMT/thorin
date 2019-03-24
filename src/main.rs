
#![allow(improper_ctypes)]

extern crate gimli;
extern crate fallible_iterator;
extern crate goblin;
extern crate libc;
#[macro_use] extern crate text_io;


use fallible_iterator::FallibleIterator;
use std::io::Read;
use std::io::Write;
use std::path::Path;
use std::process::Command;
use std::collections::HashMap;


extern {
    fn setup(child: libc::pid_t, callback: unsafe extern fn(*mut Scope, libc::uintptr_t), scope: *mut Scope);
    fn read_addr(buffer: *mut libc::c_void, address: libc::uintptr_t, size: libc::size_t);
}


#[allow(unused)]
struct Variable {
    name: String,
    offset: i64
}


#[allow(unused)]
struct Scope {
    name: Option<String>,
    variables: HashMap<String, Variable>,
    scopes: Vec<Scope>
}


macro_rules! dwarf_iter_entries {
    ($dwarf:ident, $unit:ident, $d_depth:ident, $entry:ident, $body:block) => {
        {
            let units: Vec<_> = $dwarf.units().collect().unwrap();
            for header in units {
                let $unit = match $dwarf.unit(header) {
                    Ok(r) => r,
                    Err(err) => {
                        println!("error constructing unit for header {:?}: {}", header, err);
                        continue;
                    }
                };

                let mut entries = $unit.entries();
                while let Some(($d_depth, $entry)) = entries.next_dfs().unwrap()
                    $body
            }
        }
    };
}


macro_rules! dwarf_find_attr {
    ($entry:ident, $attr_value_ident:ident, $attr_name_expr:expr, $body:block) => {
        {
            let attrs: Vec<_> = $entry.attrs().collect().unwrap();
            for attr in attrs {
                let attr_name = attr.name().static_string().unwrap();
                if attr_name == $attr_name_expr {
                    let $attr_value_ident = attr.value();
                    $body;
                    break;
                }
            }
        }
    };
}


fn construct_scope<'a>(dwarf: &'a gimli::Dwarf<gimli::EndianSlice<gimli::LittleEndian>>) -> Scope {
    let mut scope: Scope = Scope {
        name: None,
        variables: HashMap::new(),
        scopes: Vec::new()
    };

    dwarf_iter_entries!(dwarf, unit, d_depth, entry, {
        if entry.tag() != gimli::DW_TAG_variable && entry.tag() != gimli::DW_TAG_formal_parameter {
            continue;
        }

        let mut name: Option<&str> = None;
        let mut offset: Option<i64> = None;

        dwarf_find_attr!(entry, attr_value, "DW_AT_name", {
            name = Some(dwarf.attr_string(&unit, attr_value).unwrap().to_string().unwrap());
        });

        dwarf_find_attr!(entry, attr_value, "DW_AT_location", {
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
        });

        if name.is_some() && offset.is_some() {
            scope.variables.insert(
                String::from(name.unwrap()),
                Variable { name: String::from(name.unwrap()), offset: offset.unwrap() }
            );
        }
    });

    return scope;
}


fn get_types<'a>(dwarf: &'a gimli::Dwarf<gimli::EndianSlice<gimli::LittleEndian>>) -> HashMap<&'a str, u64> {
    let mut types: HashMap<&str, u64> = HashMap::new();
    types.insert("*", 8);

    dwarf_iter_entries!(dwarf, unit, d_depth, entry, {
        if entry.tag() != gimli::DW_TAG_base_type { continue; }

        let mut name: Option<&str> = None;
        let mut size: Option<u64> = None;

        dwarf_find_attr!(entry, attr_value, "DW_AT_name", {
            name = Some(dwarf.attr_string(&unit, attr_value).unwrap().to_string().unwrap());
        });

        dwarf_find_attr!(entry, attr_value, "DW_AT_byte_size", {
            if let gimli::AttributeValue::Udata(r_size) = attr_value {
                size = Some(r_size);
            }
        });

        if name.is_some() && size.is_some() {
            types.insert(name.unwrap(), size.unwrap());
        }
    });

    return types;
}


fn main() {
    // open file
    let exec_path = std::env::args().nth(1).expect("Missing argument");
    let mut dsym_path = exec_path.clone();
    dsym_path.push_str(".dSYM/Contents/Resources/DWARF/");
    dsym_path.push_str(Path::new(&exec_path).file_name().unwrap().to_str().unwrap());

    println!("loading DWARF file at {}...", dsym_path);

    let mut file = match std::fs::File::open(&dsym_path) {
        Ok(file) => file,
        Err(err) => {
            println!("Error opening file '{}': {}", dsym_path, err);
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

    let global_scope = construct_scope(&dwarf);
    let types = get_types(&dwarf);

    println!("done.");
    println!("executing program...");

    let child_pid = Command::new(exec_path).spawn().expect("failed to start program").id();
    let c_child_pid: libc::pid_t = child_pid as libc::pid_t;
    let c_scope = Box::new(global_scope);
    let c_scope_ptr: &'static mut Scope = Box::leak(c_scope);
    unsafe { setup(c_child_pid, exc_callback, &mut *c_scope_ptr); }
}


unsafe extern "C" fn exc_callback(scope: *mut Scope, rbp: libc::uintptr_t) {
    print!("inspect var: "); std::io::stdout().flush().unwrap();
    let mut varname: String = read!();
    let variables = &(*scope).variables;
    while variables.get(&varname).is_none() {
        println!("{} unrecognized.", varname);
        print!("inspect var: "); std::io::stdout().flush().unwrap();
        varname = read!();
    }

    let offset = variables.get(&varname).unwrap().offset;
    let addr = (rbp as i64) + offset;
    let result: *mut f32 = libc::malloc(std::mem::size_of::<f32>()) as *mut f32;

    read_addr(result as *mut libc::c_void, addr as libc::uintptr_t, 4);

    println!("{}: {}", &varname, *result);

    libc::free(result as *mut libc::c_void);
    Box::from_raw(scope);
}
