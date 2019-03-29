
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
#[derive(Clone, Debug)]
struct Variable {
    name: String,
    offset: i64,
    type_name: String
}


#[allow(unused)]
#[derive(Clone)]
struct Scope {
    name: Option<String>,
    variables: HashMap<String, Variable>,
    scopes: Vec<Scope>,
    low_pc: u64,
    high_pc: u64
}


#[allow(unused)]
struct DerivedType {
    name: String,
    base_types: Vec<String>
}


macro_rules! dwarf_iter_units {
    ($dwarf:ident, $unit:ident, $body:block) => {
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

                $body
            }
        }
    };
}


macro_rules! dwarf_iter_entries {
    ($dwarf:ident, $unit:ident, $d_depth:ident, $entry:ident, $body:block) => {
        {
            dwarf_iter_units!($dwarf, $unit, {
                let mut entries = $unit.entries();
                while let Some((mut $d_depth, $entry)) = entries.next_dfs().unwrap()
                    $body
            });
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


fn process_variable<'a, 'b>(
    dwarf: &'a gimli::Dwarf<gimli::EndianSlice<'b, gimli::LittleEndian>>,
    unit: &'a gimli::Unit<gimli::EndianSlice<'b, gimli::LittleEndian>>,
    node: &'a gimli::EntriesTreeNode<gimli::EndianSlice<'b, gimli::LittleEndian>>
) -> Option<Variable> {
    let entry = node.entry();
    if entry.tag() != gimli::DW_TAG_variable && entry.tag() != gimli::DW_TAG_formal_parameter {
        return None;
    }

    let mut name: Option<&str> = None;
    let mut offset: Option<i64> = None;
    let mut type_name: Option<&str> = None;

    dwarf_find_attr!(entry, attr_value, "DW_AT_name", {
        name = Some(dwarf.attr_string(unit, attr_value).unwrap().to_string().unwrap());
    });

    dwarf_find_attr!(entry, attr_value, "DW_AT_location", {
        let data = match attr_value {
            gimli::AttributeValue::Exprloc(r) => r,
            _ => { break; }
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

    dwarf_find_attr!(entry, attr_value, "DW_AT_type", {
        let u_offset = match attr_value {
            gimli::AttributeValue::UnitRef(r) => r,
            _ => { break; }
        };

        let mut t_entries = unit.entries_at_offset(u_offset).unwrap();
        let first_entry = match t_entries.next_dfs().unwrap() {
            Some((_, r)) => r,
            None => { break; }
        };

        if first_entry.tag() == gimli::DW_TAG_pointer_type {
            type_name = Some("*");
            break;
        }

        dwarf_find_attr!(first_entry, t_attr_value, "DW_AT_name", {
            type_name = Some(dwarf.attr_string(unit, t_attr_value).unwrap().to_string().unwrap());
        });
    });

    if name.is_some() && offset.is_some() {
        return Some(Variable {
            name: String::from(name.unwrap()),
            offset: offset.unwrap(),
            type_name: String::from(match type_name { Some(t) => t, None => "" })
        });
    }

    return None;
}


fn construct_scope<'a, 'b>(
    dwarf: &'a gimli::Dwarf<gimli::EndianSlice<'b, gimli::LittleEndian>>,
    unit: &'a gimli::Unit<gimli::EndianSlice<'b, gimli::LittleEndian>>,
    node: gimli::EntriesTreeNode<gimli::EndianSlice<'b, gimli::LittleEndian>>
) -> Option<Scope> {
    let mut scope = Scope {
        name: None,
        variables: HashMap::new(),
        scopes: Vec::new(),
        low_pc: 0,
        high_pc: 0
    };

    {
        let entry = node.entry();
        dwarf_find_attr!(entry, attr_value, "DW_AT_name", {
            scope.name = Some(String::from(dwarf.attr_string(unit, attr_value).unwrap().to_string().unwrap()));
        });

        dwarf_find_attr!(entry, attr_value, "DW_AT_low_pc", {
            if let gimli::AttributeValue::Addr(addr) = attr_value {
                scope.low_pc = addr;
            }
        });

        dwarf_find_attr!(entry, attr_value, "DW_AT_high_pc", {
            if let gimli::AttributeValue::Udata(addr) = attr_value {
                scope.high_pc = addr;
            }
        });
    }

    let mut children = node.children();
    while let Some(child) = children.next().unwrap() {
        let tag = child.entry().tag();
        if tag == gimli::DW_TAG_variable || tag == gimli::DW_TAG_formal_parameter {
            if let Some(var) = process_variable(dwarf, unit, &child) {
                scope.variables.insert(var.name.clone(), var);
            }
        }

        if tag == gimli::DW_TAG_subprogram || tag == gimli::DW_TAG_lexical_block {
            if let Some(s) = construct_scope(dwarf, unit, child) {
                scope.scopes.push(s);
            }
        }
    }

    return Some(scope);
}

fn construct_global_scope<'a>(
    dwarf: &'a gimli::Dwarf<gimli::EndianSlice<gimli::LittleEndian>>
) -> Scope {
    let mut global_scope = Scope {
        name: None,
        variables: HashMap::new(),
        scopes: Vec::new(),
        low_pc: 0,
        high_pc: 0
    };

    dwarf_iter_units!(dwarf, unit, {
        let mut tree = unit.entries_tree(None).unwrap();
        let root = tree.root().unwrap();
        if let Some(scope) = construct_scope(dwarf, &unit, root) {
            global_scope.scopes.push(scope);
        }
    });

    return global_scope;
}


fn get_types<'a>(dwarf: &'a gimli::Dwarf<gimli::EndianSlice<gimli::LittleEndian>>) -> HashMap<String, DerivedType> {
    let mut types: HashMap<String, DerivedType> = HashMap::new();

    dwarf_iter_entries!(dwarf, unit, d_depth, entry, {
        if entry.tag() != gimli::DW_TAG_typedef
            && entry.tag() != gimli::DW_TAG_structure_type
            && entry.tag() != gimli::DW_TAG_pointer_type {
                continue;
            }

        let mut name: Option<&str> = None;
        let mut base_types: Vec<String> = Vec::new();

        dwarf_find_attr!(entry, attr_value, "DW_AT_name", {
            name = Some(dwarf.attr_string(&unit, attr_value).unwrap().to_string().unwrap());
        });

        if name.is_some() && base_types.len() > 0 {
            types.insert(String::from(name.unwrap()), DerivedType {
                name: String::from(name.unwrap()),
                base_types: base_types
            });
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

    let global_scope = construct_global_scope(&dwarf);
    let types = get_types(&dwarf);

    print_scope("", &global_scope);

    println!("done.");
    println!("executing program...");

    let child_pid = Command::new(exec_path).spawn().expect("failed to start program").id();
    let c_child_pid: libc::pid_t = child_pid as libc::pid_t;
    let c_scope = Box::new(global_scope);
    let c_scope_ptr: &'static mut Scope = Box::leak(c_scope);
    unsafe { setup(c_child_pid, exc_callback, &mut *c_scope_ptr); }
}


fn print_scope(offset: &str, scope: &Scope) {
    println!("{}scope name: {:?}", offset, scope.name);
    println!("{}  variables: {:?}", offset, scope.variables);
    println!("{}  low_pc: {:?}", offset, scope.low_pc);
    println!("{}  high_pc: {:?}", offset, scope.high_pc);

    let mut off = String::from(offset); off.push_str("  ");
    for s in &scope.scopes {
        println!("");
        print_scope(&off, &s);
    }
}


unsafe extern "C" fn exc_callback(scope: *mut Scope, rbp: libc::uintptr_t) {
    loop {
        print!("thorin> "); std::io::stdout().flush().unwrap();
        let command_s: String = read!("{}\n");
        let command: Vec<_> = command_s.split_whitespace().collect();
        let verb = command[0].to_string();

        if verb == "exit" { break; }

        let varname = command[1].to_string();
        let variables = &(*scope).variables;
        if variables.get(&varname).is_none() {
            println!("unrecognized variable '{}'.", varname);
            continue;
        }

        let offset = variables.get(&varname).unwrap().offset;
        let type_name = &variables.get(&varname).unwrap().type_name;
        let addr = (rbp as i64) + offset;

        macro_rules! print_result_as {
            ($t:ty) => {
                {
                    let size = std::mem::size_of::<$t>();
                    let result: *mut $t = libc::malloc(size) as *mut $t;
                    read_addr(result as *mut libc::c_void, addr as libc::uintptr_t, size);
                    println!("{} {}: {}", &type_name, &varname, *result);
                    libc::free(result as *mut libc::c_void);
                }
            };

            ($t:ty, $hex:expr) => {
                {
                    let size = std::mem::size_of::<$t>();
                    let result: *mut $t = libc::malloc(size) as *mut $t;
                    read_addr(result as *mut libc::c_void, addr as libc::uintptr_t, size);
                    println!("{} {}: {:#x}", &type_name, &varname, *result);
                    libc::free(result as *mut libc::c_void);
                }
            };
        }

        match type_name.as_ref() {
            "char" | "signed char" | "unsigned char" => { print_result_as!(char); },

            "short" | "signed short" | "short int" | "signed short int" => { print_result_as!(i16); },
            "unsigned short" | "unsigned short int" => { print_result_as!(u16); },

            "int" | "signed int" | "signed" => { print_result_as!(i16); },
            "unsigned int" | "unsigned" => { print_result_as!(u16) },

            "long" | "signed long" | "long int" | "signed long int" => { print_result_as!(i32); },
            "unsigned long" | "unsigned long int" => { print_result_as!(u32); },

            "long long" | "signed long long" | "long long int" | "signed long long int" => { print_result_as!(i64); },
            "unsigned long long" | "unsigned long long int" => { print_result_as!(u64); },

            "float" => { print_result_as!(f32); },
            "double" => { print_result_as!(f64); }

            "*" => { print_result_as!(u64, true); }

            _ => { println!("unknown type"); continue; }
        }
    }

    Box::from_raw(scope);
}
