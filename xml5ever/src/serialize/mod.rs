// Copyright 2014-2017 The html5ever Project Developers. See the
// COPYRIGHT file at the top-level directory of this distribution.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

use std::collections::btree_map::BTreeMap;
use std::collections::HashMap;
use std::io::{self, Write, Error};
use std::io::ErrorKind::InvalidData;

use markup5ever::{Namespace, Prefix, EqStr};
use markup5ever::{namespace_prefix, namespace_url, ns};
pub use markup5ever::serialize::{AttrRef, Serialize, Serializer, TraversalScope};

use crate::{QualName};
use crate::util::{is_xml_char, is_name_start_char, is_name_char};

type LocalPrefixMap = HashMap<Prefix, Namespace>;

#[derive(Clone, Debug)]
/// [Namespace Prefix Map](https://w3c.github.io/DOM-Parsing/#the-namespace-prefix-map)
/// 
/// As defined in W3C Editor's Draft. `NamespacePrefixMap`, associates
/// unqiue key value that is either `Namespace` or `None` with an ordered list of associated prefix values.
/// The `NamespacePrefixMap` will be populated by previosuly seen Namespaces#
/// and all their previously encountered prefix associations for a given node and its ancestors.
/// 
/// Important preconditions: If the `NamespacePrefixMap` contains a namespace (or a None value), 
/// the attached candidates list will not be empty. Also all missing value for prefix are replaced
/// with prefix for empty string (e.g.`Prefix::from("")`).
pub struct NamespacePrefixMap {
    map: BTreeMap<Option<Namespace>, Vec<Prefix>>,
}

impl NamespacePrefixMap {
    /// Constructs a new empty `NamespacePrefixMap`
    pub fn new() -> NamespacePrefixMap {
        NamespacePrefixMap {
            map: BTreeMap::default(),
        }
    }

    /// [Retrieves preferred prefix](https://w3c.github.io/DOM-Parsing/#dfn-retrieving-a-preferred-prefix-string)
    /// 
    /// If for given namespace a candidates list exist, returns either `preferred_prefix` if found
    /// or the last `Prefix` in candidates list.alloc
    /// 
    /// If for given namespace no candidates list exist, returns `None`.
    pub fn retrieve_preferred_prefix(&self, ns: &Option<Namespace>, preferred_prefix: &Prefix) -> Option<&Prefix> {
        if let Some(candidates_list) = self.map.get(ns) {
            // Since the candidates list will always contain at least one element, this
            // will never panic.
            let mut last_prefix = &candidates_list[0];
            for prefix in candidates_list {
                last_prefix = prefix;
                if prefix == preferred_prefix {
                    break;
                }
            }
            return Some(last_prefix)
        }
        None
    }

    /// [Finds a prefix](https://w3c.github.io/DOM-Parsing/#dfn-found).
    ///
    /// If it finds any element for given namespace that matches given prefix, it will return
    /// `true` for first prefix that matches given prefix.
    ///
    /// Otherwise returns `false`.
    pub fn find_prefix(&self, ns: &Option<Namespace>, prefix: &Prefix) -> bool {
        if let Some(candidates_list) = self.map.get(ns) {
            return candidates_list.iter().any(|pref| pref == prefix)
        }
        false
    }
    /// [Adds a prefix to `NamespacePrefixMap`](https://w3c.github.io/DOM-Parsing/#dfn-add)
    /// 
    /// If there already is an ordered list associated with that `Namespace` key,
    /// it adds the Prefix to the end of the list.
    /// 
    /// Otherwise it creates an empty `list`, adds `Prefix` to the end of that `list`, 
    /// and inserts an entry with given `Namespace` as key, and `list` as value.
    pub fn add(&mut self, ns: Option<Namespace>, prefix: Prefix) {
        if let Some(candidates_list) = self.map.get_mut(&ns) {
            candidates_list.push(prefix);
        } else {
            let candidate_list = vec![prefix];
            self.map.insert(ns, candidate_list);
        }
    }

}

fn map_opt_atom<'a, T: From<&'a str> + Clone>(opt: &Option<T>) -> T {
    if let Some(atom) = opt {
        atom.clone()
    } else {
        T::from("")
    }
}



#[derive(Clone)]
/// Struct for setting serializer options.
pub struct SerializeOpts {
    /// Serialize the root node? Default: ChildrenOnly
    pub traversal_scope: TraversalScope,
    /// Flag require well-formed? Default: true
    pub require_well_formed: bool,
    /// Default context namespace of serialized document. Default: None
    pub context_namespace: Option<Namespace>,
    /// Prefix map passed to serializer. Defaults: Empty map
    pub prefix_map: NamespacePrefixMap,
    /// Generated namespace prefix index. Defaults: 1u32
    pub prefix_index: u32,
}

impl Default for SerializeOpts {
    fn default() -> SerializeOpts {
        SerializeOpts {
            traversal_scope: TraversalScope::ChildrenOnly(None),
            require_well_formed: true,
            context_namespace: None,
            prefix_map: {
                let mut namesepace_map = NamespacePrefixMap::new();
                namesepace_map.add(Some(ns!(xml)), namespace_prefix!("xml"));
                namesepace_map
            },
            prefix_index: 1,
        }
    }
}

/// Method for serializing generic node to a given writer.
pub fn serialize<Wr, T>(writer: Wr, node: &T, opts: SerializeOpts) -> io::Result<()>
where
    Wr: Write,
    T: Serialize,
{
    let traversal_scope = opts.traversal_scope.clone();
    let mut ser = XmlSerializer::new(writer, opts);
    node.serialize(&mut ser, traversal_scope)
}

/// Struct used for serializing nodes into a text that other XML
/// parses can read.
///
/// Serializer contains a set of functions (start_elem, end_elem...)
/// that make parsing nodes easier.
pub struct XmlSerializer<Wr> {
    writer: Wr,
    opts: SerializeOpts,
    skip_end_tag: bool,
}

impl<Wr: Write> XmlSerializer<Wr> {
    /// Creates a new Serializier from a writer and given serialization options.
    pub fn new(writer: Wr, opts: SerializeOpts) -> Self {

        XmlSerializer {
            writer: writer,
            opts: opts,
            skip_end_tag: false,
        }
    }

    /// Writes given text into the Serializer, escaping it,
    /// depending on where the text is written inside the tag or attribute value.
    ///
    /// For example
    ///```text
    ///    <tag>'&-quotes'</tag>   becomes      <tag>'&amp;-quotes'</tag>
    ///    <tag = "'&-quotes'">    becomes      <tag = "&apos;&amp;-quotes&apos;"
    ///```
    fn write_escaped_text(&mut self, text: &str) -> io::Result<()> {
        for c in text.chars() {
            match c {
                '&' => self.writer.write_all(b"&amp;"),
                '>' => self.writer.write_all(b"&gt;"),
                '<' => self.writer.write_all(b"&lt;"),
                c => {
                    if self.opts.require_well_formed {
                        if !is_xml_char(c){
                            return Err(Error::new(InvalidData, format!("Invalid characters = {}", c)));
                        }
                    }
                    self.writer.write_fmt(format_args!("{}", c))
                },
            }?;
        }
        Ok(())
    }


    /// [Recording namespace information](https://w3c.github.io/DOM-Parsing/#recording-the-namespace)
    /// 
    /// The following algorithm upadtes the `NamespacePrefixMap` with any found 
    /// namespace prefix defintions, adds the found prefix definition to the 
    /// local prefixes map and returns a local default namespace if there
    /// is one.
    fn recording_namespace_information<'a, AttrIter>(&self, 
            map: &mut NamespacePrefixMap,
            local_prefixes_map: &mut LocalPrefixMap,
            attrs: AttrIter) -> Option<Namespace>
    where 
        AttrIter: Iterator<Item = AttrRef<'a>>
    {
        let mut default_namespace_attr_value = None;

        for attr in attrs {
            let attribute_namespace = &attr.0.ns;
            let attribute_prefix = &attr.0.prefix;

            if attribute_namespace.eq_str("xmlns") {
                match attribute_prefix {
                    None => default_namespace_attr_value = Some(Namespace::from(attr.1)),
                    Some(_prefix) => {
                        let prefix_definition = Prefix::from(&*attr.0.local);
                        let namespace_definition = attr.1;

                        if namespace_definition == &ns!(xml) {
                            continue;
                        }

                        let namespace_definition = if namespace_definition == "" {
                            None
                        } else {
                            Some(Namespace::from(namespace_definition))
                        };

                        if map.find_prefix(&namespace_definition, &prefix_definition) {
                            continue;
                        }

                        map.add(namespace_definition.clone(), prefix_definition.clone());
                        local_prefixes_map.insert(
                            prefix_definition,
                            map_opt_atom(&namespace_definition),
                        );
                    },
                }
            }
        }
        default_namespace_attr_value
    }
}

impl<Wr: Write> Serializer for XmlSerializer<Wr> {
    
    // TODO
    fn start_elem<'a, AttrIter>(&mut self, name: QualName, attrs: AttrIter, leaf_node: bool) -> io::Result<()>
    where
        AttrIter: Iterator<Item = AttrRef<'a>> {

        // 3.2.1.1 point 1
        // Check if required well-formed flag is set.
        // If it is, check if local name contains `:` (U+003A COLON) or
        // if local name doesn't with XML name rules.
        if self.opts.require_well_formed {
            if name.local.contains(":") {
                return Err(Error::new(InvalidData, "Local name can't contain `:`."));
            }
            let mut first = true;
            for local_char in name.local.chars() {
                if first {
                    first = false;
                    if !is_name_start_char(local_char) {
                        return Err(Error::new(InvalidData, format!("Local name can't start with `{}`.", local_char)));
                    }
                } else {
                    if !is_name_char(local_char) {
                        return Err(Error::new(InvalidData, format!("Local name can't contain `{}`.", local_char)));
                    }
                }
            }
        }
        // 3.2.1.1 point 2
        // Let markup be string "<"
        self.writer.write_all(b"<")?;

        // 3.2.1.1 point 3
        let qualified_name = String::new();

        // 3.2.1.1 point 4
        self.skip_end_tag = false;

        // 3.2.1.1 point 5
        let mut ignore_namespace_definition = false;

        // 3.2.1.1 point 6
        // Given prefix map, copy a namespace prefix map into map
        let mut map = self.opts.prefix_map.clone();

        // 3.2.1.1 point 7
        // Local prefix map is an empty map, with unique Node prefix strings as keys
        // and corresponding namespaceURI Node values as the map's key values
        let mut local_prefixes_map = LocalPrefixMap::new();

        // 3.2.1.1. point 8
        let local_default_namespace = self.recording_namespace_information(&mut map, &mut local_prefixes_map, attrs);

        // 3.2.1.1. point 9
        let inherited_ns = self.opts.context_namespace.clone();

        // 3.2.1.1. point 10
        // let ns be the value of the node's namespaceURI attribute
        let ns = name.ns;

        match inherited_ns {
            // 3.2.1.1. point 11
            // If inherited_ns is equal to ns
            Some(inherited_ns) if inherited_ns == ns  =>{
                // if local default namespace is not null, then ignore namespace definition attribute
                if local_default_namespace.is_some() {
                    ignore_namespace_definition = true;
                }
                // If ns is in the XML namespace 
                // then append to qualified name "xml"
                // else append node's local name, node prefix is dropped.
                if ns == ns!(xml) {
                    self.writer.write_all(b"xml:")?;
                    self.writer.write_all(name.local.as_bytes())?;
                } else {
                    self.writer.write_all(name.local.as_bytes())?;
                }
            },
             // 3.2.1.1. point 12
            // Otherwise, inherited_ns is not equal to ns (the node's
            // own namespace definition)
            _ => {
                let prefix = name.prefix;
                let candidate_prefix = map.retrieve_preferred_prefix(&inherited_ns, &map_opt_atom(&prefix));

                if prefix.eq_str("xmlns") {
                    if self.opts.require_well_formed {
                        return Err(Error::new(InvalidData, format!("An Element with prefix 'xmlns' will not legally round-trip in a conforming XML parser. ")));
                    }
                }
                //TODO
            }
        }



        self.writer.write_all(b">")
    }


    // TODO
    fn end_elem(&mut self, name: QualName) -> io::Result<()> {
        Ok(())
    }

    // TODO Until DTD is fully implementeds
    fn write_doctype(&mut self, text: &str) -> io::Result<()> {
        self.writer.write_all(b"<!DOCTYPE ")?;
        self.writer.write_all(text.as_bytes())?;
        self.writer.write_all(b"!>")
    }

    fn write_text(&mut self, text: &str) -> io::Result<()> {
        self.write_escaped_text(text)
    }

    fn write_processing_instruction(&mut self, target: &str, data: &str) -> io::Result<()> {
        if self.opts.require_well_formed {
            if "xml".eq_ignore_ascii_case(target) {
                return Err(Error::new(InvalidData, "Processing instruction target can't be equal to `xml`."));  
            }
            if target.contains(':') {
                return Err(Error::new(InvalidData, "Processing instruction target contains illegal characters `:`.")); 
            }
            if data.contains("?>") {
                return Err(Error::new(InvalidData, "Processing instruction data contains illegal characters `?>`."))
            }
        }
        self.writer.write_all(b"<?")?;
        self.write_escaped_text(target)?;
        self.writer.write_all(b" ")?;
        self.write_escaped_text(data)?;
        self.writer.write_all(b"?>")
    }

    fn write_comment(&mut self, text: &str) -> io::Result<()> {
        if self.opts.require_well_formed {
            if text.contains("--") {
                return Err(Error::new(InvalidData, "Comment contains double minus `--`")); 
            } else if text.ends_with("-") {
                return Err(Error::new(InvalidData, "Comment ends with minus"));
            }
        }
        self.writer.write_all(b"<!--")?;
        self.write_escaped_text(text)?;
        self.writer.write_all(b"-->")
    }
}
