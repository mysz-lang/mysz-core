use crate::parse::parsing::Type;

pub fn mangle_name(base_name: &str, args: &[Type]) -> String {
    let mut name = base_name.to_string();
    for arg in args {
        name.push_str("__");
        name.push_str(&type_to_mangled_string(arg));
    }
    name
}

pub fn type_to_mangled_string(ty: &Type) -> String {
    match ty {
        Type::Int => "int".to_string(),
        Type::UInt => "uint".to_string(),
        Type::Int8 => "int8".to_string(),
        Type::UInt8 => "uint8".to_string(),
        Type::Bool => "bool".to_string(),
        Type::Str => "str".to_string(),
        Type::Char => "char".to_string(),
        Type::Void => "void".to_string(),
        Type::Any => "any".to_string(),
        Type::Struct(name) => name.clone(),
        Type::Ptr(inner) => format!("ptr__{}", type_to_mangled_string(inner)),
        Type::Array { element_type, size } => {
            format!("arr__{}__{}", type_to_mangled_string(element_type), size)
        }
        Type::GenericInstance { name, args } => {
            let mut base = name.clone();
            for arg in args {
                base.push_str("__");
                base.push_str(&type_to_mangled_string(arg));
            }
            base
        }
        Type::GenericParam(s) => format!("gparam__{}", s),
    }
}

pub fn normalise_type(ty: &Type) -> Type {
    match ty {
        Type::GenericInstance { name, args } => {
            let mangled_name = mangle_name(name, args);
            Type::Struct(mangled_name)
        }
        Type::Ptr(inner) => Type::Ptr(Box::new(normalise_type(inner))),
        Type::Array { element_type, size } => Type::Array {
            element_type: Box::new(normalise_type(element_type)),
            size: *size,
        },
        _ => ty.clone(),
    }
}

#[inline]
pub fn is_integer(ty: &Type) -> bool {
    matches!(ty, Type::Int | Type::UInt | Type::Int8 | Type::UInt8)
}

#[inline]
pub fn is_signed_integer(ty: &Type) -> bool {
    matches!(ty, Type::Int)
}

#[inline]
pub fn is_truthy_type(ty: &Type) -> bool {
    matches!(
        ty,
        Type::Int | Type::UInt | Type::Int8 | Type::UInt8 | Type::Bool | Type::Str
    )
}

pub fn types_compatible(expected: &Type, from: &Type) -> bool {
    if expected == &Type::Any || from == &Type::Any {
        return true;
    }

    let norm_expected = normalise_type(expected);
    let norm_from = normalise_type(from);

    if norm_expected == norm_from {
        return true;
    }

    if is_integer(&norm_from) && is_integer(&norm_expected) {
        return true;
    }

    if norm_from == Type::Ptr(Box::new(Type::Char)) && norm_expected == Type::Str {
        return true;
    }

    if norm_expected == Type::Ptr(Box::new(Type::Char)) && norm_from == Type::Str {
        return true;
    }

    false
}

pub fn type_to_string(ty: &Type) -> String {
    match ty {
        Type::Int => "int".to_string(),
        Type::UInt => "uint".to_string(),
        Type::Int8 => "i8".to_string(),
        Type::UInt8 => "u8".to_string(),
        Type::Bool => "bool".to_string(),
        Type::Str => "str".to_string(),
        Type::Char => "char".to_string(),
        Type::Void => "void".to_string(),
        Type::Any => "any".to_string(),
        Type::Struct(name) => name.clone(),
        Type::Ptr(inner) => format!("ptr<{}>", type_to_string(inner)),
        Type::Array { element_type, size } => {
            format!("[{}; {}]", type_to_string(element_type), size)
        }
        Type::GenericInstance { name, args } => {
            let args_str: Vec<String> = args.iter().map(|arg| type_to_string(arg)).collect();
            format!("{}<{}>", name, args_str.join(", "))
        }
        Type::GenericParam(s) => format!("<{}>", s),
    }
}
