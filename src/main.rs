
#![allow(improper_ctypes)]

// thorin/main.rs
//
// The thorin debugger -- it ain't much, but it's honest work.
//
// author: Ajay Tatachar <ajaymt2@illinois.edu>

extern crate gimli;
extern crate fallible_iterator;
extern crate object;
extern crate memmap;
extern crate libc;
extern crate hex;
extern crate rand;
#[macro_use] extern crate text_io;


use object::Object;
use object::ObjectSection;
use fallible_iterator::FallibleIterator;
use std::io::Write;
use std::path::Path;
use std::collections::HashMap;
use rand::Rng;


// these are the C functions defined in thorin.c that do all of the actual
// system-call stuff.
// Since rust forbids global mutable state, we need to route the `scope` and `types`
// globals through the C code.
extern {
    fn setup(
        child: *const std::os::raw::c_char,
        callback: unsafe extern fn(*mut Scope, *mut HashMap<String, DerivedType>, libc::uintptr_t, libc::uintptr_t),
        scope: *mut Scope,
        types: *mut HashMap<String, DerivedType>
    );
    fn read_addr(buffer: *mut libc::c_void, address: libc::uintptr_t, size: libc::size_t);
}


// A variable has a name, an offset from the stack base pointer or struct base
// and a type name.
#[allow(unused)]
#[derive(Clone, Debug)]
struct Variable {
    name: String,
    offset: i64,
    type_name: String
}


// A scope has an optional name (if it is a function), a set of variables, a set
// of child scopes, and a program counter range to find the scope within the target
// process (based on the instruction pointer, i.e RIP register since we're on x86_64)
#[allow(unused)]
#[derive(Clone)]
struct Scope {
    name: Option<String>,
    variables: HashMap<String, Variable>,
    scopes: Vec<Scope>,
    low_pc: u64,
    high_pc: u64
}


// A derived type is either a typedef or struct. It has a name and one of a base
// type (for typedefs) or list of members (for structs)
#[allow(unused)]
struct DerivedType {
    name: String,
    base_type: String,
    members: Vec<Variable>
}


// this macro iterates through compilation units in a DWARF file
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


// this macro iterates through DIEs (debugging information entries) in all compilation
// units in a DWARF file
macro_rules! dwarf_iter_entries {
    ($dwarf:ident, $unit:ident, $d_depth:ident, $entry:ident, $body:block) => {
        {
            dwarf_iter_units!($dwarf, $unit, {
                let mut entries = $unit.entries();
                while let Some(($d_depth, $entry)) = entries.next_dfs().unwrap()
                    $body
            });
        }
    };
}


// this macro finds a specific DWARF attribute in a DIE
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


// this function constructs a Variable struct out of a DIE
fn process_variable<'a, 'b>(
    dwarf: &'a gimli::Dwarf<gimli::EndianSlice<'b, gimli::LittleEndian>>,
    unit: &'a gimli::Unit<gimli::EndianSlice<'b, gimli::LittleEndian>>,
    node: &'a gimli::EntriesTreeNode<gimli::EndianSlice<'b, gimli::LittleEndian>>
) -> Option<Variable> {
    let entry = node.entry();
    if entry.tag() != gimli::DW_TAG_variable
        && entry.tag() != gimli::DW_TAG_formal_parameter
        && entry.tag() != gimli::DW_TAG_member {
            return None;
        }

    let mut name: Option<&str> = None;
    let mut offset: Option<i64> = None;
    let mut type_name: Option<&str> = None;

    dwarf_find_attr!(entry, attr_value, "DW_AT_name", {
        name = Some(dwarf.attr_string(unit, attr_value).unwrap().to_string().unwrap());
    });

    if entry.tag() == gimli::DW_TAG_member {
        dwarf_find_attr!(entry, attr_value, "DW_AT_data_member_location", {
            if let gimli::AttributeValue::Udata(addr) = attr_value {
                offset = Some(addr as i64);
            }
        });
    } else {
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
    }

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


// this function recursively constructs a scope out of a set of DIEs
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
        high_pc: std::u64::MAX
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


// this function constructs the global scope struct starting from the root DIE
fn construct_global_scope<'a>(
    dwarf: &'a gimli::Dwarf<gimli::EndianSlice<gimli::LittleEndian>>
) -> Scope {
    let mut global_scope = Scope {
        name: Some(String::from("root")),
        variables: HashMap::new(),
        scopes: Vec::new(),
        low_pc: 0,
        high_pc: std::u64::MAX
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


// this function constructs the set of derived types from the root DIEs
fn get_types<'a>(dwarf: &'a gimli::Dwarf<gimli::EndianSlice<gimli::LittleEndian>>) -> HashMap<String, DerivedType> {
    let mut types: HashMap<String, DerivedType> = HashMap::new();

    dwarf_iter_entries!(dwarf, unit, _d_depth, entry, {
        if entry.tag() != gimli::DW_TAG_typedef
            && entry.tag() != gimli::DW_TAG_structure_type
            && entry.tag() != gimli::DW_TAG_pointer_type {
                continue;
            }

        let mut name: Option<&str> = None;
        let mut base_type: Option<&str> = None;
        let mut members: Vec<Variable> = Vec::new();

        dwarf_find_attr!(entry, attr_value, "DW_AT_name", {
            name = Some(dwarf.attr_string(&unit, attr_value).unwrap().to_string().unwrap());
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
                base_type = Some("*");
                break;
            }

            dwarf_find_attr!(first_entry, t_attr_value, "DW_AT_name", {
                base_type = Some(dwarf.attr_string(&unit, t_attr_value).unwrap().to_string().unwrap());
            });
        });

        if entry.tag() == gimli::DW_TAG_structure_type {
            let mut tree = unit.entries_tree(Some(entry.offset())).unwrap();
            let root = tree.root().unwrap();
            let mut children = root.children();
            while let Some(child) = children.next().unwrap() {
                let tag = child.entry().tag();
                if tag != gimli::DW_TAG_member { continue };

                let member = process_variable(dwarf, &unit, &child);
                if let Some(r) = member { members.push(r); }
            }
        }

        if name.is_some() && (base_type.is_some() || members.len() > 0) {
            types.insert(String::from(name.unwrap()), DerivedType {
                name: String::from(name.unwrap()),
                base_type: String::from(if let Some(s) = base_type { s } else { "" }),
                members: members
            });
        }
    });

    return types;
}


// this is the entry point of the program
fn main() {
    let exec_path = std::env::args().nth(1).expect("Missing argument");
    let mut dsym_path = exec_path.clone();
    #[cfg(target_os = "macos")]
    {
        dsym_path.push_str(".dSYM/Contents/Resources/DWARF/");
        dsym_path.push_str(Path::new(&exec_path).file_name().unwrap().to_str().unwrap());
    }

    println!("loading DWARF file at {}...", dsym_path);
    let file = match std::fs::File::open(&dsym_path) {
        Ok(file) => file,
        Err(err) => {
            println!("Error opening file '{}': {}", &dsym_path, err);
            return;
        }
    };
    let mmapped_file = match unsafe { memmap::Mmap::map(&file) } {
        Ok(mmapped_file) => mmapped_file,
        Err(err) => {
            println!("Could not map file '{}': {}", &dsym_path, err);
            return;
        }
    };
    let parsed_file = match object::File::parse(&*mmapped_file) {
        Ok(parsed_file) => parsed_file,
        Err(err) => {
            println!("Error parsing file '{}': {}", &dsym_path, err);
            return;
        }
    };

    macro_rules! load_section {
        ($x:ident, $y:ident) => (
            gimli::$x::new(
                &$y,
                gimli::LittleEndian
            );
        )
    }

    let s_debug_info = parsed_file.section_by_name(".debug_info")
        .expect("No .debug_info section found")
        .data();
    let s_debug_abbrev = parsed_file.section_by_name(".debug_abbrev")
        .expect("No .debug_abbrev section found")
        .data();
    let s_debug_str = parsed_file.section_by_name(".debug_str")
        .expect("No .debug_str section found")
        .data();
    let s_debug_line = parsed_file.section_by_name(".debug_line")
        .expect("No .debug_line section found")
        .data();
    let debug_info = load_section!(DebugInfo, s_debug_info);
    let debug_abbrev = load_section!(DebugAbbrev, s_debug_abbrev);
    let debug_line = load_section!(DebugLine, s_debug_line);
    let debug_str = load_section!(DebugStr, s_debug_str);
    let dwarf = gimli::Dwarf {
        debug_info,
        debug_abbrev,
        debug_line,
        debug_str,
        ..Default::default()
    };

    let global_scope = construct_global_scope(&dwarf);
    let types = get_types(&dwarf);

    println!("done.");
    println!("executing {}...\n", exec_path);

    let exec_path_c = std::ffi::CString::new(String::from(exec_path)).unwrap();
    let c_scope = Box::new(global_scope);
    let c_scope_ptr: &'static mut Scope = Box::leak(c_scope);
    let c_types = Box::new(types);
    let c_types_ptr: &'static mut HashMap<String, DerivedType> = Box::leak(c_types);
    unsafe { setup(exec_path_c.as_ptr(), exc_callback, &mut *c_scope_ptr, &mut *c_types_ptr); }
}


// this function tries to find which scope we're inside of in the suspended
// target process based on the instruction pointer and the low_pc and high_pc
// attributes of the scope structs constructed earlier
// takes O(logn) time because the scope struct is essentially a
// searchable BTree -- CS225 ftw :)
fn construct_context(
    scope: &Scope,
    variables: &mut HashMap<String, Variable>,
    scopes: &mut Vec<String>,
    rip: u64
) {
    if let Some(ref name) = scope.name {
        scopes.push(name.clone());
    } else {
        scopes.push(String::from("unnamed scope"));
    }

    for (name, val) in &(scope.variables) {
        variables.insert(name.clone(), val.clone());
    }

    for child in &(scope.scopes) {
        if rip >= child.low_pc && rip - child.low_pc <= child.high_pc {
            construct_context(child, variables, scopes, rip);
        }
    }
}


// this macro reads and prints a variable at a specific address in the child process
macro_rules! print_result_as {
    ($t:ty, $addr:ident) => {
        {
            let size = std::mem::size_of::<$t>();
            let result: *mut $t = libc::malloc(size) as *mut $t;
            read_addr(result as *mut libc::c_void, $addr as libc::uintptr_t, size);
            println!("{}", *result);
            libc::free(result as *mut libc::c_void);
        }
    };

    ($t:ty, $addr:ident, $hex:ident) => {
        {
            let size = std::mem::size_of::<$t>();
            let result: *mut $t = libc::malloc(size) as *mut $t;
            read_addr(result as *mut libc::c_void, $addr as libc::uintptr_t, size);
            println!("{:#x}", *result);
            libc::free(result as *mut libc::c_void);
        }
    };

    ($t:ty, $addr:ident, $count:expr, $zero:expr) => {
        {
            let size = std::mem::size_of::<$t>() * $count;
            let mut result: Vec<$t> = vec![$zero; $count];
            {
                let slice: &mut [$t] = &mut result;
                read_addr(slice.as_mut_ptr() as *mut libc::c_void, $addr as libc::uintptr_t, size);
            }
            println!("{:?}", result);
        }
    };
}


// this macro resolves the (base) type of a variable and prints it
macro_rules! print_base_type {
    ($type_name:ident, $addr:ident, $count:expr) => {
        match $type_name {
            "char" | "signed char" | "unsigned char" => {
                if $count == 1 { print_result_as!(libc::c_char, $addr); }
                else { print_result_as!(libc::c_char, $addr, ($count), 0); }
            },

            "short" | "signed short" | "short int" | "signed short int" | "short signed" | "short signed int" => {
                if $count == 1 { print_result_as!(i16, $addr); }
                else { print_result_as!(i16, $addr, ($count), 0); }
            },
            "unsigned short" | "unsigned short int" | "short unsigned" | "short unsigned int" => {
                if $count == 1 { print_result_as!(u16, $addr); }
                else { print_result_as!(u16, $addr, ($count), 0); }
            },

            "int" | "signed int" | "signed" => {
                if $count == 1 { print_result_as!(i16, $addr); }
                else { print_result_as!(i16, $addr, ($count), 0); }
            },
            "unsigned int" | "unsigned" => {
                if $count == 1 { print_result_as!(u16, $addr); }
                else { print_result_as!(u16, $addr, ($count), 0); }
            },

            "long" | "signed long" | "long int" | "signed long int" | "long signed" | "long signed int" => {
                if $count == 1 { print_result_as!(i32, $addr); }
                else { print_result_as!(i32, $addr, ($count), 0); }
            },
            "unsigned long" | "unsigned long int" | "long unsigned" | "long unsigned int" => {
                if $count == 1 { print_result_as!(u32, $addr); }
                else { print_result_as!(u32, $addr, ($count), 0); }
            },

            "long long" | "signed long long" | "long long int" | "signed long long int" | "long long signed" | "long long signed int" => {
                if $count == 1 { print_result_as!(i64, $addr); }
                else { print_result_as!(i64, $addr, ($count), 0); }
            },
            "unsigned long long" | "unsigned long long int" | "long long unsigned" | "long long unsigned int" => {
                if $count == 1 { print_result_as!(u64, $addr); }
                else { print_result_as!(u64, $addr, ($count), 0); }
            },

            "float" => {
                if $count == 1 { print_result_as!(f32, $addr); }
                else { print_result_as!(f32, $addr, ($count), 0.0); }
            },
            "double" => {
                if $count == 1 { print_result_as!(f64, $addr); }
                else { print_result_as!(f64, $addr, ($count), 0.0); }
            }

            "*" => {
                if $count == 1 { print_result_as!(u64, $addr, $addr); }
                else { print_result_as!(u64, $addr, ($count), 0); }
            }

            _ => { println!("unknown type"); }
        }
    };
}


// this macro recursively reolves the (derived) type of a variable and prints it
fn print_struct(offset: &str, varname: &str, type_name: &str, addr: i64, types: &HashMap<String, DerivedType>) {
    print!("{}{} {}: ", offset, type_name, varname);
    let d_type = types.get(type_name);
    if let Some(ref dt) = d_type {
        println!("");
        let new_offset = format!("  {}", offset);
        if dt.members.len() > 0 {
            for member in &dt.members {
                let new_addr = addr + member.offset;
                print_struct(&new_offset, &member.name, &member.type_name, new_addr, types);
            }
        } else {
            print_struct(&new_offset, varname, &dt.base_type, addr, types);
        }
    } else {
        unsafe { print_base_type!(type_name, addr, 1); }
    }
}


// this function reads an arbitrary address in the child process as a specific type and prints the results
unsafe fn read_ptr(address: u64, count: usize, type_name: &str, types: &HashMap<String, DerivedType>) {
    let d_type = types.get(type_name);

    if d_type.is_none() {
        print_base_type!(type_name, address, (count));
    } else {
        if d_type.unwrap().members.len() > 0 {
            println!("cannot read structs yet"); return;
        }
        read_ptr(address, count, &d_type.unwrap().base_type, types);
    }
}


// this is the exception callback -- it gets called when the target process is suspended
// and starts the main debugger loop
unsafe extern "C" fn exc_callback(
    scope_p: *mut Scope,
    types_p: *mut HashMap<String, DerivedType>,
    rbp: libc::uintptr_t,
    rip: libc::uintptr_t
) {
    println!("Process suspended.\n");

    let mut variables: HashMap<String, Variable> = HashMap::new();
    let mut scopes: Vec<String> = Vec::new();
    let scope = &(*scope_p);
    construct_context(scope, &mut variables, &mut scopes, rip as u64);

    let types = &(*types_p);

    println!("Scope tree:");
    let mut scope_print_offset = String::from("");
    for scope_name in scopes {
        println!("{}-> {}", scope_print_offset, scope_name);
        scope_print_offset.push_str("  ");
    }

    println!("\nVariables defined in this scope:");
    for (key, value) in &variables {
        println!("  {}: {}", key, value.type_name);
    }

    println!("");
    loop {
        print!("thorin> "); std::io::stdout().flush().unwrap();
        let command_s: String = read!("{}\n");
        let command: Vec<_> = command_s.split_whitespace().collect();
        let verb = command[0].to_string();

        match verb.as_ref() {
            "exit" | "quit" => { break; },
            "help" => {
                println!("Commands:");
                println!("  (print|show|get) <variable-name>:  Print the value of a variable.");
                println!("  read <address> <count> <type>:     Read the value at <address>. <type>");
                println!("                                     is the type of the value, <count> is the");
                println!("                                     number of values to read.");
                println!("  help:                              Print this help message.");
                println!("  (exit|quit):                       Quit thorin.");

                continue;
            },
            "print" | "show" | "get" => {
                if command.len() < 2 {
                    println!("command '{}' expects at least one argument", verb);
                    println!("Usage: {} <variable-name>", verb);
                    continue;
                }
            },
            "read" => {
                if command.len() < 4 {
                    println!("command '{}' expects at least three arguments", verb);
                    println!("Usage: {} <address> <count> <type>", verb);
                    continue;
                }

                let address_str = command[1].trim_start_matches("0x");
                let address = match u64::from_str_radix(&address_str, 16) {
                    Ok(r) => r,
                    Err(err) => {
                        println!("error parsing address: {}", err);
                        continue;
                    }
                };
                let count_str = command[2];
                let count = match usize::from_str_radix(&count_str, 10) {
                    Ok(r) => r,
                    Err(err) => {
                        println!("error parsing count: {}", err);
                        continue;
                    }
                };
                let type_name = command[3].to_string();

                read_ptr(address, count, &type_name, &types);

                continue;
            }

            other => { println!("unknown command '{}'", other); continue; }
        }

        let varname = command[1].to_string();
        if variables.get(&varname).is_none() {
            println!("unrecognized variable '{}'.", varname);
            continue;
        }

        let offset = variables.get(&varname).unwrap().offset;
        let type_name = &variables.get(&varname).unwrap().type_name;
        let addr = (rbp as i64) + offset;

        print_struct("", &varname, &type_name, addr, &types);
    }

    println!("");
    let mut rng = rand::thread_rng();
    match rng.gen_range(0, 4) {
        0 => { println!("\"If more people valued home, above gold, this world would be a merrier place...\""); },
        1 => { println!("\"We are sons of Durin. And Durin's Folk do not flee from a fight.\""); },
        2 => { println!("\"Those who have lived through dragon fire should rejoice. They have much to be grateful for.\""); },
        _ => { println!("\"If this is to end in fire, then we will all burn together.\""); }
    }
    println!("");

    Box::from_raw(scope_p);
    Box::from_raw(types_p);
}
