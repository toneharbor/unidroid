use crate::Emulator;

pub(in crate::jni) struct Class {
    pub(in crate::jni) superclass: Option<&'static str>,
    interfaces: &'static [&'static str],
    owned: bool,
    android: bool,
}

macro_rules! classes {
    ($name:expr; $( $class:literal => $superclass:expr, $interfaces:expr, $owned:literal, $android:literal; )*) => {
        match $name {
            $($class => Class {
                superclass: $superclass,
                interfaces: &$interfaces,
                owned: $owned,
                android: $android,
            },)*
            _ => return None,
        }
    };
}

pub(in crate::jni) fn class(name: &str, android: bool) -> Option<Class> {
    // Class => superclass, interfaces, emulator-owned objects, requires an APK.
    let row = classes! { name;
        "java/lang/String" => Some("java/lang/Object"),
            ["java/io/Serializable", "java/lang/Comparable", "java/lang/CharSequence", "java/lang/constant/Constable", "java/lang/constant/ConstantDesc"], true, false;
        "java/lang/Class" => Some("java/lang/Object"),
            ["java/io/Serializable", "java/lang/reflect/GenericDeclaration", "java/lang/reflect/Type", "java/lang/reflect/AnnotatedElement", "java/lang/invoke/TypeDescriptor$OfField", "java/lang/constant/Constable"], true, false;
        "java/lang/reflect/Method" => Some("java/lang/reflect/Executable"), [], true, false;
        "java/lang/reflect/Constructor" => Some("java/lang/reflect/Executable"), [], true, false;
        "java/lang/reflect/Executable" => Some("java/lang/reflect/AccessibleObject"), [], true, false;
        "java/lang/reflect/Field" => Some("java/lang/reflect/AccessibleObject"), [], true, false;
        "java/lang/reflect/AccessibleObject" => Some("java/lang/Object"), [], true, false;
        "java/lang/reflect/GenericDeclaration" => None, ["java/lang/reflect/AnnotatedElement"], false, false;
        "java/lang/invoke/TypeDescriptor$OfField" => None, ["java/lang/invoke/TypeDescriptor"], false, false;
        "java/lang/Boolean" => Some("java/lang/Object"),
            ["java/io/Serializable", "java/lang/Comparable", "java/lang/constant/Constable"], true, false;
        "java/lang/Character" => Some("java/lang/Object"),
            ["java/io/Serializable", "java/lang/Comparable", "java/lang/constant/Constable"], true, false;
        "java/lang/Byte" => Some("java/lang/Number"), ["java/lang/Comparable", "java/lang/constant/Constable"], true, false;
        "java/lang/Short" => Some("java/lang/Number"), ["java/lang/Comparable", "java/lang/constant/Constable"], true, false;
        "java/lang/Integer" => Some("java/lang/Number"),
            ["java/lang/Comparable", "java/lang/constant/Constable", "java/lang/constant/ConstantDesc"], true, false;
        "java/lang/Long" => Some("java/lang/Number"),
            ["java/lang/Comparable", "java/lang/constant/Constable", "java/lang/constant/ConstantDesc"], true, false;
        "java/lang/Float" => Some("java/lang/Number"),
            ["java/lang/Comparable", "java/lang/constant/Constable", "java/lang/constant/ConstantDesc"], true, false;
        "java/lang/Double" => Some("java/lang/Number"),
            ["java/lang/Comparable", "java/lang/constant/Constable", "java/lang/constant/ConstantDesc"], true, false;
        "java/lang/Number" => Some("java/lang/Object"), ["java/io/Serializable"], false, false;
        "java/lang/StringBuilder" => Some("java/lang/AbstractStringBuilder"), ["java/io/Serializable", "java/lang/Comparable"], true, false;
        "java/lang/AbstractStringBuilder" => Some("java/lang/Object"), ["java/lang/Appendable", "java/lang/CharSequence"], false, false;
        "java/nio/charset/Charset" => Some("java/lang/Object"), ["java/lang/Comparable"], true, false;
        "java/security/MessageDigest" => Some("java/security/MessageDigestSpi"), [], true, false;
        "java/security/MessageDigestSpi" => Some("java/lang/Object"), [], false, false;
        "java/util/HashMap" => Some("java/util/AbstractMap"),
            ["java/util/Map", "java/lang/Cloneable", "java/io/Serializable"], true, false;
        "java/util/LinkedHashMap" => Some("java/util/HashMap"), ["java/util/SequencedMap"], true, false;
        "java/util/TreeMap" => Some("java/util/AbstractMap"),
            ["java/util/NavigableMap", "java/lang/Cloneable", "java/io/Serializable"], true, false;
        "java/util/AbstractMap" => Some("java/lang/Object"), ["java/util/Map"], false, false;
        "java/util/NavigableMap" => None, ["java/util/SortedMap"], false, false;
        "java/util/SortedMap" => None, ["java/util/SequencedMap"], false, false;
        "java/util/SequencedMap" => None, ["java/util/Map"], false, false;
        "java/util/HashSet" => Some("java/util/AbstractSet"),
            ["java/util/Set", "java/lang/Cloneable", "java/io/Serializable"], true, false;
        "java/util/ArrayList" => Some("java/util/AbstractList"),
            ["java/util/List", "java/util/RandomAccess", "java/lang/Cloneable", "java/io/Serializable"], true, false;
        "java/util/LinkedList" => Some("java/util/AbstractSequentialList"),
            ["java/util/List", "java/util/Deque", "java/lang/Cloneable", "java/io/Serializable"], true, false;
        "java/util/AbstractSequentialList" => Some("java/util/AbstractList"), [], false, false;
        "java/util/AbstractList" => Some("java/util/AbstractCollection"), ["java/util/List"], false, false;
        "java/util/AbstractSet" => Some("java/util/AbstractCollection"), ["java/util/Set"], false, false;
        "java/util/AbstractCollection" => Some("java/lang/Object"), ["java/util/Collection"], false, false;
        "java/util/List" => None, ["java/util/SequencedCollection"], false, false;
        "java/util/Set" => None, ["java/util/Collection"], false, false;
        "java/util/Queue" => None, ["java/util/Collection"], false, false;
        "java/util/SequencedCollection" => None, ["java/util/Collection"], false, false;
        "java/util/Deque" => None, ["java/util/Queue", "java/util/SequencedCollection"], false, false;
        "java/util/Collection" => None, ["java/lang/Iterable"], false, false;
        "java/util/NavigableSet" => None, ["java/util/SortedSet"], false, false;
        "java/util/SortedSet" => None, ["java/util/SequencedSet"], false, false;
        "java/util/SequencedSet" => None, ["java/util/Set", "java/util/SequencedCollection"], false, false;
        "java/util/HashMap$Node" => Some("java/lang/Object"), ["java/util/Map$Entry"], true, false;
        "java/util/TreeMap$Entry" => Some("java/lang/Object"), ["java/util/Map$Entry"], true, false;
        "java/util/LinkedHashMap$Entry" => Some("java/util/HashMap$Node"), [], true, false;
        "java/util/HashMap$KeySet" => Some("java/util/AbstractSet"), [], true, false;
        "java/util/HashMap$EntrySet" => Some("java/util/AbstractSet"), [], true, false;
        "java/util/TreeMap$EntrySet" => Some("java/util/AbstractSet"), [], true, false;
        "java/util/LinkedHashMap$LinkedKeySet" => Some("java/util/AbstractSet"), ["java/util/SequencedSet"], true, false;
        "java/util/LinkedHashMap$LinkedEntrySet" => Some("java/util/AbstractSet"), ["java/util/SequencedSet"], true, false;
        "java/util/TreeMap$KeySet" => Some("java/util/AbstractSet"), ["java/util/NavigableSet"], true, false;
        "java/util/HashMap$Values" => Some("java/util/AbstractCollection"), [], true, false;
        "java/util/TreeMap$Values" => Some("java/util/AbstractCollection"), [], true, false;
        "java/util/LinkedHashMap$LinkedValues" => Some("java/util/AbstractCollection"), ["java/util/SequencedCollection"], true, false;
        "java/util/ArrayList$Itr" => Some("java/lang/Object"), ["java/util/Iterator"], true, false;
        "java/util/LinkedList$ListItr" => Some("java/lang/Object"), ["java/util/ListIterator"], true, false;
        "java/util/ListIterator" => None, ["java/util/Iterator"], false, false;
        "java/util/HashMap$KeyIterator" => Some("java/util/HashMap$HashIterator"), ["java/util/Iterator"], true, false;
        "java/util/HashMap$ValueIterator" => Some("java/util/HashMap$HashIterator"), ["java/util/Iterator"], true, false;
        "java/util/HashMap$EntryIterator" => Some("java/util/HashMap$HashIterator"), ["java/util/Iterator"], true, false;
        "java/util/LinkedHashMap$LinkedKeyIterator" => Some("java/util/LinkedHashMap$LinkedHashIterator"), ["java/util/Iterator"], true, false;
        "java/util/LinkedHashMap$LinkedValueIterator" => Some("java/util/LinkedHashMap$LinkedHashIterator"), ["java/util/Iterator"], true, false;
        "java/util/LinkedHashMap$LinkedEntryIterator" => Some("java/util/LinkedHashMap$LinkedHashIterator"), ["java/util/Iterator"], true, false;
        "java/util/TreeMap$KeyIterator" => Some("java/util/TreeMap$PrivateEntryIterator"), [], true, false;
        "java/util/TreeMap$ValueIterator" => Some("java/util/TreeMap$PrivateEntryIterator"), [], true, false;
        "java/util/TreeMap$EntryIterator" => Some("java/util/TreeMap$PrivateEntryIterator"), [], true, false;
        "java/util/TreeMap$PrivateEntryIterator" => Some("java/lang/Object"), ["java/util/Iterator"], false, false;
        "java/util/HashMap$HashIterator" => Some("java/lang/Object"), [], false, false;
        "java/util/LinkedHashMap$LinkedHashIterator" => Some("java/lang/Object"), [], false, false;
        "java/lang/Object" => None, [], false, false;
        "java/io/Serializable" => None, [], false, false;
        "java/lang/Comparable" => None, [], false, false;
        "java/lang/CharSequence" => None, [], false, false;
        "java/lang/constant/Constable" => None, [], false, false;
        "java/lang/constant/ConstantDesc" => None, [], false, false;
        "java/lang/reflect/Type" => None, [], false, false;
        "java/lang/reflect/AnnotatedElement" => None, [], false, false;
        "java/lang/invoke/TypeDescriptor" => None, [], false, false;
        "java/lang/Appendable" => None, [], false, false;
        "java/lang/Cloneable" => None, [], false, false;
        "java/lang/Iterable" => None, [], false, false;
        "java/util/Map" => None, [], false, false;
        "java/util/Map$Entry" => None, [], false, false;
        "java/util/Iterator" => None, [], false, false;
        "java/util/RandomAccess" => None, [], false, false;
        "android/app/Application" => Some("android/content/ContextWrapper"), [], false, true;
        "android/app/Service" => Some("android/content/ContextWrapper"), [], false, true;
        "android/view/ContextThemeWrapper" => Some("android/content/ContextWrapper"), [], false, true;
        "android/app/Activity" => Some("android/view/ContextThemeWrapper"), [], false, true;
        "android/content/ContextWrapper" => Some("android/content/Context"), [], false, true;
        "android/app/ApplicationPackageManager" => Some("android/content/pm/PackageManager"), [], false, true;
        "android/content/Context" => Some("java/lang/Object"), [], false, true;
        "android/content/pm/PackageManager" => Some("java/lang/Object"), [], false, true;
        "android/content/pm/PackageInfo" => Some("java/lang/Object"), [], false, true;
        "android/content/pm/Signature" => Some("java/lang/Object"), [], false, true;
        "android/content/pm/SigningInfo" => Some("java/lang/Object"), [], false, true;
        "android/content/pm/ApplicationInfo" => Some("java/lang/Object"), [], false, true;
        "android/content/res/AssetManager" => Some("java/lang/Object"), [], false, true;
    };
    (!row.android || android).then_some(row)
}

pub(in crate::jni) fn assignable(emu: &Emulator, from: &str, to: &str) -> Option<bool> {
    let android = emu.apk().is_some();
    if !to.starts_with("java/")
        && !to.starts_with("javax/")
        && !(android && to.starts_with("android/"))
    {
        return None;
    }
    let builtins: Vec<_> = emu
        .class_chain(from)
        .into_iter()
        .filter(|name| class(name, android).is_some_and(|row| row.owned || row.android))
        .collect();
    if builtins.is_empty() {
        return None;
    }
    Some(builtins.into_iter().any(|name| reaches(name, to, android)))
}

fn reaches(from: &str, to: &str, android: bool) -> bool {
    from == to
        || class(from, android).is_some_and(|row| {
            row.superclass
                .into_iter()
                .chain(row.interfaces.iter().copied())
                .any(|parent| reaches(parent, to, android))
        })
}
