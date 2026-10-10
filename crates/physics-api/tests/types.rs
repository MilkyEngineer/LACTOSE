//! The scalar (`README.md`, "The scalar"): no component of this crate holds a raw `f32` or
//! `f64` anywhere in its layout, every hashed one registers with the world hasher, and the
//! policies are the ones the README's consumer table states.

#![forbid(unsafe_code)]

use lactose_physics_api::{
    BodyHandle, Collider, Joint, JointHandle, RigidBody, Teleported, Transform, Velocity,
    register_types,
};
use lactose_reflect::{FieldKind, Persistence, Reflect, StructInfo, TypeInfo};
use lactose_sim::TypeTable;

fn raw_floats(layout: &StructInfo, path: &str, found: &mut Vec<String>) {
    for field in layout.fields() {
        let path = format!("{path}.{}", field.name());
        let mut kind = field.kind();
        while let FieldKind::Array { element, .. } = kind {
            kind = *element;
        }
        match kind {
            FieldKind::Float => found.push(path),
            FieldKind::Struct(inner) => raw_floats(inner, &path, found),
            _ => {}
        }
    }
}

fn infos() -> [&'static TypeInfo; 8] {
    [
        Transform::SCHEMA.info(),
        Velocity::SCHEMA.info(),
        Teleported::SCHEMA.info(),
        RigidBody::SCHEMA.info(),
        Collider::SCHEMA.info(),
        Joint::SCHEMA.info(),
        BodyHandle::SCHEMA.info(),
        JointHandle::SCHEMA.info(),
    ]
}

#[test]
fn no_component_holds_a_raw_float() {
    let mut found = Vec::new();
    for info in infos() {
        raw_floats(info.layout(), info.name(), &mut found);
    }
    assert_eq!(found, Vec::<String>::new());
    // The world hasher refuses a hashed type with a raw float or a foreign scalar tag.
    register_types(&mut TypeTable::new()).unwrap();
}

#[test]
fn the_handles_are_derived_and_the_rest_persistent() {
    for info in infos() {
        let derived = info.name().ends_with("handle");
        let expected = if derived { Persistence::Derived } else { Persistence::Persistent };
        assert_eq!(info.persistence(), expected, "{}", info.name());
    }
}
