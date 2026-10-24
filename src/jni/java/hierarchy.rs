use crate::Emulator;

pub(in crate::jni) fn assignable(emu: &Emulator, from: &str, to: &str) -> Option<bool> {
    let android = emu.apk().is_some();
    if !to.starts_with("java/")
        && !to.starts_with("javax/")
        && !(android && to.starts_with("android/"))
    {
        return None;
    }
    let chain = emu.class_chain(from);
    let builtins: Vec<_> = chain
        .into_iter()
        .filter(|class| {
            owned_class(class) || (android && super::super::android::parent(class).is_some())
        })
        .collect();
    if builtins.is_empty() {
        return None;
    }
    Some(
        builtins
            .into_iter()
            .any(|class| reaches(class, to, android)),
    )
}
fn reaches(from: &str, to: &str, android: bool) -> bool {
    from == to
        || parents(from)
            .iter()
            .any(|parent| reaches(parent, to, android))
        || (android
            && super::super::android::parent(from)
                .is_some_and(|parent| reaches(parent, to, android)))
}
fn owned_class(class: &str) -> bool {
    matches!(
        class,
        "java/lang/String"
            | "java/lang/Class"
            | "java/lang/reflect/Method"
            | "java/lang/reflect/Constructor"
            | "java/lang/reflect/Field"
            | "java/lang/reflect/Executable"
            | "java/lang/reflect/AccessibleObject"
            | "java/lang/StringBuilder"
            | "java/lang/Boolean"
            | "java/lang/Byte"
            | "java/lang/Character"
            | "java/lang/Short"
            | "java/lang/Integer"
            | "java/lang/Long"
            | "java/lang/Float"
            | "java/lang/Double"
            | "java/nio/charset/Charset"
            | "java/security/MessageDigest"
            | "java/util/HashMap"
            | "java/util/LinkedHashMap"
            | "java/util/TreeMap"
            | "java/util/HashSet"
            | "java/util/ArrayList"
            | "java/util/LinkedList"
            | "java/util/HashMap$Node"
            | "java/util/LinkedHashMap$Entry"
            | "java/util/TreeMap$Entry"
            | "java/util/HashMap$KeySet"
            | "java/util/HashMap$Values"
            | "java/util/HashMap$EntrySet"
            | "java/util/LinkedHashMap$LinkedKeySet"
            | "java/util/LinkedHashMap$LinkedValues"
            | "java/util/LinkedHashMap$LinkedEntrySet"
            | "java/util/TreeMap$KeySet"
            | "java/util/TreeMap$Values"
            | "java/util/TreeMap$EntrySet"
            | "java/util/ArrayList$Itr"
            | "java/util/LinkedList$ListItr"
            | "java/util/HashMap$KeyIterator"
            | "java/util/HashMap$ValueIterator"
            | "java/util/HashMap$EntryIterator"
            | "java/util/LinkedHashMap$LinkedKeyIterator"
            | "java/util/LinkedHashMap$LinkedValueIterator"
            | "java/util/LinkedHashMap$LinkedEntryIterator"
            | "java/util/TreeMap$KeyIterator"
            | "java/util/TreeMap$ValueIterator"
            | "java/util/TreeMap$EntryIterator"
    )
}
fn parents(class: &str) -> &'static [&'static str] {
    match class {
        "java/lang/String" => &[
            "java/lang/Object",
            "java/io/Serializable",
            "java/lang/Comparable",
            "java/lang/CharSequence",
            "java/lang/constant/Constable",
            "java/lang/constant/ConstantDesc",
        ],
        "java/lang/Class" => &[
            "java/lang/Object",
            "java/io/Serializable",
            "java/lang/reflect/GenericDeclaration",
            "java/lang/reflect/Type",
            "java/lang/reflect/AnnotatedElement",
            "java/lang/invoke/TypeDescriptor$OfField",
            "java/lang/constant/Constable",
        ],
        "java/lang/reflect/Method" | "java/lang/reflect/Constructor" => {
            &["java/lang/reflect/Executable"]
        }
        "java/lang/reflect/Executable" | "java/lang/reflect/Field" => {
            &["java/lang/reflect/AccessibleObject"]
        }
        "java/lang/reflect/AccessibleObject" => &["java/lang/Object"],
        "java/lang/reflect/GenericDeclaration" => &["java/lang/reflect/AnnotatedElement"],
        "java/lang/invoke/TypeDescriptor$OfField" => &["java/lang/invoke/TypeDescriptor"],
        "java/lang/Boolean" | "java/lang/Character" => &[
            "java/lang/Object",
            "java/io/Serializable",
            "java/lang/Comparable",
            "java/lang/constant/Constable",
        ],
        "java/lang/Byte" | "java/lang/Short" => &[
            "java/lang/Number",
            "java/lang/Comparable",
            "java/lang/constant/Constable",
        ],
        "java/lang/Integer" | "java/lang/Long" | "java/lang/Float" | "java/lang/Double" => &[
            "java/lang/Number",
            "java/lang/Comparable",
            "java/lang/constant/Constable",
            "java/lang/constant/ConstantDesc",
        ],
        "java/lang/Number" => &["java/lang/Object", "java/io/Serializable"],
        "java/lang/StringBuilder" => &[
            "java/lang/AbstractStringBuilder",
            "java/io/Serializable",
            "java/lang/Comparable",
        ],
        "java/lang/AbstractStringBuilder" => &[
            "java/lang/Object",
            "java/lang/Appendable",
            "java/lang/CharSequence",
        ],
        "java/nio/charset/Charset" => &["java/lang/Object", "java/lang/Comparable"],
        "java/security/MessageDigest" => &["java/security/MessageDigestSpi"],
        "java/security/MessageDigestSpi" => &["java/lang/Object"],
        "java/util/HashMap" => &[
            "java/util/AbstractMap",
            "java/util/Map",
            "java/lang/Cloneable",
            "java/io/Serializable",
        ],
        "java/util/LinkedHashMap" => &["java/util/HashMap", "java/util/SequencedMap"],
        "java/util/TreeMap" => &[
            "java/util/AbstractMap",
            "java/util/NavigableMap",
            "java/lang/Cloneable",
            "java/io/Serializable",
        ],
        "java/util/AbstractMap" => &["java/lang/Object", "java/util/Map"],
        "java/util/NavigableMap" => &["java/util/SortedMap"],
        "java/util/SortedMap" => &["java/util/SequencedMap"],
        "java/util/SequencedMap" => &["java/util/Map"],
        "java/util/HashSet" => &[
            "java/util/AbstractSet",
            "java/util/Set",
            "java/lang/Cloneable",
            "java/io/Serializable",
        ],
        "java/util/ArrayList" => &[
            "java/util/AbstractList",
            "java/util/List",
            "java/util/RandomAccess",
            "java/lang/Cloneable",
            "java/io/Serializable",
        ],
        "java/util/LinkedList" => &[
            "java/util/AbstractSequentialList",
            "java/util/List",
            "java/util/Deque",
            "java/lang/Cloneable",
            "java/io/Serializable",
        ],
        "java/util/AbstractSequentialList" => &["java/util/AbstractList"],
        "java/util/AbstractList" => &["java/util/AbstractCollection", "java/util/List"],
        "java/util/AbstractSet" => &["java/util/AbstractCollection", "java/util/Set"],
        "java/util/AbstractCollection" => &["java/lang/Object", "java/util/Collection"],
        "java/util/List" => &["java/util/SequencedCollection"],
        "java/util/Set" | "java/util/Queue" | "java/util/SequencedCollection" => {
            &["java/util/Collection"]
        }
        "java/util/Deque" => &["java/util/Queue", "java/util/SequencedCollection"],
        "java/util/Collection" => &["java/lang/Iterable"],
        "java/util/NavigableSet" => &["java/util/SortedSet"],
        "java/util/SortedSet" => &["java/util/SequencedSet"],
        "java/util/SequencedSet" => &["java/util/Set", "java/util/SequencedCollection"],
        "java/util/HashMap$Node" | "java/util/TreeMap$Entry" => {
            &["java/lang/Object", "java/util/Map$Entry"]
        }
        "java/util/LinkedHashMap$Entry" => &["java/util/HashMap$Node"],
        "java/util/HashMap$KeySet"
        | "java/util/HashMap$EntrySet"
        | "java/util/TreeMap$EntrySet" => &["java/util/AbstractSet"],
        "java/util/LinkedHashMap$LinkedKeySet" | "java/util/LinkedHashMap$LinkedEntrySet" => {
            &["java/util/AbstractSet", "java/util/SequencedSet"]
        }
        "java/util/TreeMap$KeySet" => &["java/util/AbstractSet", "java/util/NavigableSet"],
        "java/util/HashMap$Values" | "java/util/TreeMap$Values" => {
            &["java/util/AbstractCollection"]
        }
        "java/util/LinkedHashMap$LinkedValues" => &[
            "java/util/AbstractCollection",
            "java/util/SequencedCollection",
        ],
        "java/util/ArrayList$Itr" => &["java/lang/Object", "java/util/Iterator"],
        "java/util/LinkedList$ListItr" => &["java/lang/Object", "java/util/ListIterator"],
        "java/util/ListIterator" => &["java/util/Iterator"],
        "java/util/HashMap$KeyIterator"
        | "java/util/HashMap$ValueIterator"
        | "java/util/HashMap$EntryIterator" => {
            &["java/util/HashMap$HashIterator", "java/util/Iterator"]
        }
        "java/util/LinkedHashMap$LinkedKeyIterator"
        | "java/util/LinkedHashMap$LinkedValueIterator"
        | "java/util/LinkedHashMap$LinkedEntryIterator" => &[
            "java/util/LinkedHashMap$LinkedHashIterator",
            "java/util/Iterator",
        ],
        "java/util/TreeMap$KeyIterator"
        | "java/util/TreeMap$ValueIterator"
        | "java/util/TreeMap$EntryIterator" => &["java/util/TreeMap$PrivateEntryIterator"],
        "java/util/TreeMap$PrivateEntryIterator" => &["java/lang/Object", "java/util/Iterator"],
        "java/util/HashMap$HashIterator" | "java/util/LinkedHashMap$LinkedHashIterator" => {
            &["java/lang/Object"]
        }
        _ => &[],
    }
}
