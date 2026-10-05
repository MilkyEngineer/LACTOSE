//! The attributes, metadata, visitors, registry and byte access, from the position of an
//! invoking crate: `unsafe` is forbidden here, as it is in nearly every crate that declares
//! a component.

#![forbid(unsafe_code)]
#![deny(warnings, missing_docs)]

use lactose_core::{
    ComponentKey, EntityRef, ErrorKind, FieldId, NetId, PlainArray, PlayerId, StableKey, Tick,
};
use lactose_reflect::bytemuck;
use lactose_reflect::{
    Component, Field, FieldInfo, FieldKind, FieldVisitor, Padding, Persistence, ReferenceVisitor,
    Reflect, Registry, Resource, StructInfo, Tracking, TypeInfo, TypeRole, component, decode,
    resource, value,
};

/// What a character is doing.
#[value]
#[repr(u8)]
#[derive(PartialEq, Eq, Debug)]
pub enum Stance {
    /// On the ground.
    Standing = 0,
    /// On a wall.
    Climbing = 1,
    /// In water.
    Swimming = 4,
}

/// One contact.
#[value]
#[derive(PartialEq, Debug)]
pub struct Hit {
    /// Who was hit.
    #[field(id = 1)]
    pub target: EntityRef,
    /// How hard.
    #[field(id = 2)]
    pub amount: u16,
    /// Whether it was blocked.
    #[field(id = 3)]
    pub blocked: bool,
    #[field(padding)]
    _pad: Padding<1>,
}

/// Hit points.
#[component(key = "test::health", persistent, version = 3, replicated, extracted)]
#[derive(PartialEq, Debug)]
pub struct Health(#[field(id = 1)] pub i32);

/// A tag.
#[component(key = "test::player_tag", persistent)]
pub struct PlayerTag;

/// Everything a field may be.
#[component(key = "test::kitchen_sink", persistent, replicated)]
#[derive(PartialEq, Debug)]
pub struct KitchenSink {
    /// Sixty-four bits first: the author orders by alignment.
    #[field(id = 1)]
    pub origin: Option<NetId>,
    /// A reference.
    #[field(id = 2)]
    pub held: EntityRef,
    /// An array.
    #[field(id = 7)]
    pub cooldowns: PlainArray<u32, 2>,
    /// A tick.
    #[field(id = 3)]
    pub since: Tick,
    /// A player.
    #[field(id = 4)]
    pub owner: PlayerId,
    /// A presentation-side float.
    #[field(id = 9)]
    pub fade: f32,
    /// An enum.
    #[field(id = 5)]
    pub stance: Stance,
    /// A flag.
    #[field(id = 6)]
    pub grounded: bool,
    #[field(padding)]
    _pad: Padding<2>,
}

/// Contacts this tick: an inline array plus a length.
#[component(key = "test::hit_list", transient)]
pub struct HitList {
    /// The contacts; slots at and past `len` are zero.
    #[field(id = 1)]
    pub hits: PlainArray<Hit, 3>,
    /// The number of contacts.
    #[field(id = 2)]
    pub len: u32,
}

/// Gravity.
#[resource(key = "test::gravity", persistent, replicated)]
pub struct Gravity {
    /// Millimetres per tick squared.
    #[field(id = 1)]
    pub down: i64,
}

fn net(bits: u64) -> EntityRef {
    EntityRef::new(StableKey::Net(NetId::from_bits(bits).unwrap()))
}

fn sink() -> KitchenSink {
    KitchenSink {
        origin: NetId::from_bits(77),
        held: net(5),
        cooldowns: PlainArray([10, 20]),
        since: Tick(9),
        owner: PlayerId(2),
        fade: 0.5,
        stance: Stance::Swimming,
        grounded: true,
        _pad: Padding::default(),
    }
}

fn hit_list() -> HitList {
    let hit = |target, amount| Hit { target, amount, blocked: false, _pad: Padding::default() };
    HitList { hits: PlainArray([hit(net(1), 3), hit(EntityRef::NONE, 0), hit(net(2), 4)]), len: 3 }
}

fn registry() -> Registry {
    let mut registry = Registry::new();
    registry.register::<Health>().unwrap();
    registry.register::<PlayerTag>().unwrap();
    registry.register::<KitchenSink>().unwrap();
    registry.register::<HitList>().unwrap();
    registry.register::<Gravity>().unwrap();
    registry
}

fn is_component<T: Component>() {}
fn is_resource<T: Resource>() {}

#[test]
fn attributes_implement_the_role_traits() {
    is_component::<Health>();
    is_component::<PlayerTag>();
    is_component::<HitList>();
    is_resource::<Gravity>();
}

#[test]
fn key_is_the_hash_of_the_declared_name() {
    const KEY: ComponentKey = Health::KEY;
    assert_eq!(KEY, ComponentKey::from_name("test::health"));
    assert_eq!(Health::INFO.key(), KEY);
    assert_eq!(Health::INFO.name(), "test::health");
}

#[test]
fn metadata_states_policy_version_tracking_and_role() {
    let health = Health::INFO;
    assert_eq!(health.role(), TypeRole::Component);
    assert_eq!(health.persistence(), Persistence::Persistent);
    assert_eq!(health.version(), 3);
    assert_eq!(health.tracking(), Tracking { replicated: true, extracted: true });

    let hits = HitList::INFO;
    assert_eq!(hits.persistence(), Persistence::Transient);
    assert_eq!(hits.version(), 1);
    assert_eq!(hits.tracking(), Tracking::default());

    assert_eq!(Gravity::INFO.role(), TypeRole::Resource);
    assert_eq!(Gravity::INFO.tracking(), Tracking { replicated: true, extracted: false });
}

#[test]
fn layout_comes_from_the_compiler() {
    let layout = KitchenSink::INFO.layout();
    assert_eq!(layout.name(), "KitchenSink");
    assert_eq!(layout.size(), size_of::<KitchenSink>());
    assert_eq!(layout.align(), align_of::<KitchenSink>());

    let described: Vec<_> = layout
        .fields()
        .iter()
        .map(|f| (f.id().map(|id| id.0), f.name(), f.offset(), f.size()))
        .collect();
    assert_eq!(
        described,
        [
            (Some(1), "origin", 0, 8),
            (Some(2), "held", 8, 16),
            (Some(7), "cooldowns", 24, 8),
            (Some(3), "since", 32, 4),
            (Some(4), "owner", 36, 4),
            (Some(9), "fade", 40, 4),
            (Some(5), "stance", 44, 1),
            (Some(6), "grounded", 45, 1),
            (None, "_pad", 46, 2),
        ]
    );
    assert!(layout.fields()[8].is_padding());

    let tuple = Health::INFO.layout().fields();
    assert_eq!((tuple[0].name(), tuple[0].offset(), tuple[0].size()), ("0", 0, 4));
    assert!(PlayerTag::INFO.layout().fields().is_empty());
    assert_eq!(PlayerTag::INFO.layout().size(), 0);
}

#[test]
fn kinds_come_from_the_field_trait() {
    let kind = |id| KitchenSink::INFO.field(FieldId(id)).unwrap().kind();
    assert!(matches!(kind(1), FieldKind::Unsigned));
    assert!(matches!(kind(2), FieldKind::EntityRef));
    assert!(matches!(kind(7), FieldKind::Array { element: FieldKind::Unsigned, len: 2 }));
    assert!(matches!(kind(9), FieldKind::Float));
    assert!(matches!(kind(6), FieldKind::Bool));
    let FieldKind::Enum(stance) = kind(5) else { panic!("stance is an enum") };
    assert_eq!((stance.name, stance.size), ("Stance", 1));
    let variants: Vec<_> = stance.variants.iter().map(|v| (v.name, v.value)).collect();
    assert_eq!(variants, [("Standing", 0), ("Climbing", 1), ("Swimming", 4)]);

    let FieldKind::Array { element: FieldKind::Struct(hit), len: 3 } =
        HitList::INFO.field(FieldId(1)).unwrap().kind()
    else {
        panic!("hits is an array of structs")
    };
    assert_eq!((hit.name(), hit.size()), ("Hit", size_of::<Hit>()));
    assert!(hit.fields()[3].is_padding());
    assert!(matches!(<i16 as Field>::KIND, FieldKind::Signed));
}

/// Records what a hasher would see, proving each kind arrives through its own method.
#[derive(Default)]
struct Recorder(Vec<String>);

impl FieldVisitor for Recorder {
    fn bytes(&mut self, id: FieldId, bytes: &[u8]) {
        self.0.push(format!("{} bytes {bytes:?}", id.0));
    }

    fn f32(&mut self, id: FieldId, value: f32) {
        self.0.push(format!("{} f32 {value}", id.0));
    }

    fn f64(&mut self, id: FieldId, value: f64) {
        self.0.push(format!("{} f64 {value}", id.0));
    }

    fn entity_ref(&mut self, id: FieldId, value: EntityRef) {
        self.0.push(format!("{} entity {:?}", id.0, value.get()));
    }

    fn asset_ref<A: Field>(&mut self, id: FieldId, _: &A) {
        self.0.push(format!("{} asset", id.0));
    }
}

#[test]
fn visit_reports_every_non_padding_field_by_kind_in_declaration_order() {
    let mut recorder = Recorder::default();
    sink().visit(&mut recorder);
    assert_eq!(
        recorder.0,
        [
            "1 bytes [77, 0, 0, 0, 0, 0, 0, 0]",
            "2 entity Some(Net(NetId(5)))",
            "7 bytes [10, 0, 0, 0]",
            "7 bytes [20, 0, 0, 0]",
            "3 bytes [9, 0, 0, 0]",
            "4 bytes [2, 0, 0, 0]",
            "9 f32 0.5",
            "5 bytes [4]",
            "6 bytes [1]",
        ]
    );
}

#[test]
fn nested_values_report_their_leaves_under_the_outer_field_id() {
    let mut recorder = Recorder::default();
    hit_list().visit(&mut recorder);
    assert_eq!(recorder.0.len(), 3 * 3 + 1);
    assert_eq!(recorder.0[0], "1 entity Some(Net(NetId(1)))");
    assert_eq!(recorder.0[1], "1 bytes [3, 0]");
    assert_eq!(recorder.0[2], "1 bytes [0]");
    assert_eq!(recorder.0[3], "1 entity None");
    assert_eq!(recorder.0[9], "2 bytes [3, 0, 0, 0]");

    let mut recorder = Recorder::default();
    PlayerTag.visit(&mut recorder);
    assert!(recorder.0.is_empty());
}

/// Adds a fixed amount to every network ID, as a load-time remap would.
struct Shift(u64, Vec<u16>);

impl ReferenceVisitor for Shift {
    fn entity_ref(&mut self, id: FieldId, value: &mut EntityRef) {
        self.1.push(id.0);
        if let Some(StableKey::Net(net_id)) = value.get() {
            *value = net(net_id.to_bits() + self.0);
        }
    }

    fn asset_ref<A: Field>(&mut self, _: FieldId, _: &mut A) {}
}

#[test]
fn references_are_rewritten_in_place() {
    let mut sink = sink();
    let mut shift = Shift(100, Vec::new());
    sink.visit_references(&mut shift);
    assert_eq!(sink.held, net(105));
    assert_eq!(shift.1, [2]);
    // Nothing else moved.
    assert_eq!(KitchenSink { held: net(5), ..sink }, self::sink());

    let mut hits = hit_list();
    let mut shift = Shift(10, Vec::new());
    hits.visit_references(&mut shift);
    assert_eq!(shift.1, [1, 1, 1]);
    assert_eq!(hits.hits.map(|hit| hit.target), [net(11), EntityRef::NONE, net(12)]);
}

#[test]
fn registry_finds_types_by_key_and_by_rust_type() {
    let registry = registry();
    assert_eq!(registry.len(), 5);
    assert!(!registry.is_empty());
    assert!(registry.contains::<Health>());
    let by_key = registry.get(Health::KEY).unwrap();
    assert_eq!(by_key.name(), "test::health");
    let by_type = registry.get_by_type(core::any::TypeId::of::<Gravity>()).unwrap();
    assert_eq!(by_type.name(), "test::gravity");
    assert!(registry.get(ComponentKey(1)).is_none());
    assert!(registry.get_by_type(core::any::TypeId::of::<u32>()).is_none());
}

#[test]
fn registration_is_idempotent_and_order_does_not_change_iteration() {
    let forward = registry();
    let mut backward = Registry::new();
    backward.register::<Gravity>().unwrap();
    backward.register::<HitList>().unwrap();
    backward.register::<KitchenSink>().unwrap();
    backward.register::<PlayerTag>().unwrap();
    backward.register::<Health>().unwrap();
    backward.register::<Health>().unwrap();
    assert_eq!(backward.len(), 5);

    let keys: Vec<_> = forward.iter().map(TypeInfo::key).collect();
    assert!(keys.is_sorted());
    assert_eq!(keys, backward.iter().map(TypeInfo::key).collect::<Vec<_>>());
    assert_eq!(forward.dump(), backward.dump());
}

#[test]
fn dump_lists_every_type_and_field() {
    let dump = registry().dump();
    let health = format!(
        "component test::health key={:#018x} version=3 persistent size=4 replicated extracted\n  \
         1 0: i32 size=4\n",
        Health::KEY.0
    );
    assert!(dump.contains(&health), "{dump}");
    assert!(dump.contains("resource test::gravity key="), "{dump}");
    assert!(dump.contains("  1 down: i64 size=8\n"), "{dump}");
    assert!(dump.contains("  1 origin: u64 size=8\n"), "{dump}");
    assert!(dump.contains("  7 cooldowns: [u32; 2] size=8\n"), "{dump}");
    assert!(
        dump.contains(
            "  5 stance: enum Stance { Standing = 0, Climbing = 1, Swimming = 4 } size=1\n"
        ),
        "{dump}"
    );
    assert!(dump.contains("  - _pad: padding size=2\n"), "{dump}");
    assert!(
        dump.contains(
            "  1 hits: [struct Hit { 1 target: entity_ref size=16, 2 amount: u16 size=2, \
             3 blocked: bool size=1, - _pad: padding size=1 }; 3] size=60\n"
        ),
        "{dump}"
    );
    assert_eq!(dump.lines().filter(|line| !line.starts_with(' ')).count(), 5);
}

/// A second type under the name `test::health`.
#[component(key = "test::health", derived)]
pub struct HealthImpostor;

macro_rules! hand_written {
    ($(#[$doc:meta])* $name:ident, $info:expr, key = $key:expr) => {
        $(#[$doc])*
        #[derive(Clone, Copy, bytemuck::NoUninit, bytemuck::CheckedBitPattern)]
        #[repr(C)]
        pub struct $name {
            a: u32,
            b: u32,
        }

        impl Reflect for $name {
            const KEY: ComponentKey = $key;
            const INFO: &'static TypeInfo = &$info;

            fn visit<V: FieldVisitor>(&self, _: &mut V) {}

            fn visit_references<V: ReferenceVisitor>(&mut self, _: &mut V) {}
        }
    };
}

const fn info(name: &'static str, layout: StructInfo) -> TypeInfo {
    TypeInfo::new(
        name,
        TypeRole::Component,
        Persistence::Persistent,
        1,
        Tracking { replicated: false, extracted: false },
        layout,
    )
}

const fn two_fields(a: u16, b: u16, b_offset: usize) -> [FieldInfo; 2] {
    [FieldInfo::new::<u32>(FieldId(a), "a", 0), FieldInfo::new::<u32>(FieldId(b), "b", b_offset)]
}

hand_written!(
    /// Its key constant is not the hash of its name.
    WrongKey,
    info("test::wrong_key", StructInfo::new::<WrongKey>("WrongKey", &two_fields(1, 2, 4))),
    key = ComponentKey(7)
);
hand_written!(
    /// Uses one field ID twice.
    DuplicateId,
    info("test::duplicate_id", StructInfo::new::<DuplicateId>("DuplicateId", &two_fields(1, 1, 4))),
    key = ComponentKey::from_name("test::duplicate_id")
);
hand_written!(
    /// Describes the second field at the wrong offset.
    WrongOffset,
    info("test::wrong_offset", StructInfo::new::<WrongOffset>("WrongOffset", &two_fields(1, 2, 2))),
    key = ComponentKey::from_name("test::wrong_offset")
);
hand_written!(
    /// Describes only the first field.
    MissingField,
    info(
        "test::missing_field",
        StructInfo::new::<MissingField>("MissingField", &[FieldInfo::new::<u32>(FieldId(1), "a", 0)])
    ),
    key = ComponentKey::from_name("test::missing_field")
);
hand_written!(
    /// Describes another type's layout.
    WrongSize,
    info("test::wrong_size", StructInfo::new::<u32>("WrongSize", &[FieldInfo::new::<u32>(FieldId(1), "a", 0)])),
    key = ComponentKey::from_name("test::wrong_size")
);

/// Zeroed every tick, but zero is not a network ID.
#[component(key = "test::bad_transient", transient)]
pub struct BadTransient(#[field(id = 1)] pub NetId);

#[test]
fn registration_rejects_what_contradicts_the_type() {
    fn rejected<T: Reflect>(expected: &str) {
        let mut registry = Registry::new();
        let error = registry.register::<T>().unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidData);
        assert!(error.message().contains(expected), "{error}");
        assert!(registry.is_empty());
    }
    rejected::<WrongKey>("key is not the hash of its name");
    rejected::<DuplicateId>("field ID 1 is used by both `a` and `b`");
    rejected::<WrongOffset>("field `b` is at offset 2 but the previous field ends at 4");
    rejected::<MissingField>("fields cover 4 bytes of a 8-byte struct");
    rejected::<WrongSize>("described as 4 bytes aligned to 4, but the type is 8 aligned to 4");
    rejected::<BadTransient>("its all-zero value is invalid");
}

#[test]
fn registration_rejects_a_second_type_under_a_registered_key() {
    let mut registry = registry();
    let error = registry.register::<HealthImpostor>().unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidData);
    assert!(error.message().contains("`test::health` is declared by two types"), "{error}");
    assert_eq!(registry.len(), 5);
    assert_eq!(registry.get(Health::KEY).unwrap().version(), 3);
}

#[test]
fn field_bytes_are_read_by_id() {
    let info = KitchenSink::INFO;
    let sink = sink();
    let bytes = bytemuck::bytes_of(&sink);
    assert_eq!(info.field_bytes(bytes, FieldId(3)).unwrap(), 9u32.to_le_bytes());
    assert_eq!(info.field_bytes(bytes, FieldId(5)).unwrap(), [4]);

    let unknown = info.field_bytes(bytes, FieldId(100)).unwrap_err();
    assert_eq!(unknown.kind(), ErrorKind::NotFound);
    let short = info.field_bytes(&bytes[1..], FieldId(3)).unwrap_err();
    assert_eq!(short.kind(), ErrorKind::InvalidData);
}

#[test]
fn field_writes_are_validated_and_leave_the_value_alone_on_error() {
    let info = KitchenSink::INFO;
    let mut bytes = bytemuck::bytes_of(&sink()).to_vec();

    info.write_field(&mut bytes, FieldId(5), &[1]).unwrap();
    info.write_field(&mut bytes, FieldId(3), &40u32.to_le_bytes()).unwrap();
    let written: KitchenSink = decode(&bytes).unwrap();
    assert_eq!(written, KitchenSink { stance: Stance::Climbing, since: Tick(40), ..sink() });

    let before = bytes.clone();
    let invalid = [
        info.write_field(&mut bytes, FieldId(5), &[2]),
        info.write_field(&mut bytes, FieldId(6), &[7]),
        info.write_field(&mut bytes, FieldId(3), &[0; 3]),
        // A placeholder reference is never valid in stored bytes.
        info.write_field(&mut bytes, FieldId(2), bytemuck::bytes_of(&[3u32, 1, 0, 0])),
    ];
    for result in invalid {
        assert_eq!(result.unwrap_err().kind(), ErrorKind::InvalidData);
    }
    assert_eq!(
        info.write_field(&mut bytes, FieldId(8), &[0]).unwrap_err().kind(),
        ErrorKind::NotFound
    );
    assert_eq!(bytes, before);

    // An array field is valid only if every element is.
    let info = HitList::INFO;
    let mut bytes = bytemuck::bytes_of(&hit_list()).to_vec();
    let mut hits = info.field_bytes(&bytes, FieldId(1)).unwrap().to_vec();
    hits[18] = 2;
    assert_eq!(
        info.write_field(&mut bytes, FieldId(1), &hits).unwrap_err().kind(),
        ErrorKind::InvalidData
    );
    hits[18] = 1;
    info.write_field(&mut bytes, FieldId(1), &hits).unwrap();
    assert!(decode::<HitList>(&bytes).unwrap().hits[0].blocked);
}

#[test]
fn whole_values_are_validated_without_panicking() {
    let info = KitchenSink::INFO;
    let mut bytes = bytemuck::bytes_of(&sink()).to_vec();
    info.validate(&bytes).unwrap();
    assert_eq!(decode::<KitchenSink>(&bytes).unwrap(), sink());

    assert_eq!(info.validate(&bytes[..4]).unwrap_err().kind(), ErrorKind::InvalidData);
    assert_eq!(decode::<KitchenSink>(&bytes[..4]).unwrap_err().kind(), ErrorKind::InvalidData);

    bytes[44] = 3;
    assert_eq!(info.validate(&bytes).unwrap_err().kind(), ErrorKind::InvalidData);
    assert_eq!(decode::<KitchenSink>(&bytes).unwrap_err().kind(), ErrorKind::InvalidData);

    // Every byte value in every position: an error or a value, never a panic.
    let clean = bytemuck::bytes_of(&sink()).to_vec();
    for position in 0..clean.len() {
        for byte in [0x00, 0x01, 0x02, 0x7f, 0x80, 0xff] {
            let mut bytes = clean.clone();
            bytes[position] = byte;
            assert_eq!(info.validate(&bytes).is_ok(), decode::<KitchenSink>(&bytes).is_ok());
        }
    }
}

#[test]
fn padding_is_zero_and_is_zeroed() {
    assert_eq!(size_of::<Padding<3>>(), 3);
    assert_eq!(bytemuck::bytes_of(&Padding::<3>::default()), [0; 3]);

    let info = KitchenSink::INFO;
    let mut bytes = bytemuck::bytes_of(&sink()).to_vec();
    bytes[47] = 9;
    assert_eq!(info.validate(&bytes).unwrap_err().kind(), ErrorKind::InvalidData);
    info.zero_padding(&mut bytes).unwrap();
    assert_eq!(decode::<KitchenSink>(&bytes).unwrap(), sink());

    // Padding inside a nested struct inside an array.
    let info = HitList::INFO;
    let clean = bytemuck::bytes_of(&hit_list()).to_vec();
    let mut bytes = clean.clone();
    for element in 0..3 {
        bytes[element * 20 + 19] = 0xee;
    }
    assert!(info.validate(&bytes).is_err());
    info.zero_padding(&mut bytes).unwrap();
    assert_eq!(bytes, clean);
    assert!(info.zero_padding(&mut bytes[1..]).is_err());
}

#[test]
fn option_net_id_is_a_field_with_zero_as_none() {
    let info = KitchenSink::INFO;
    let mut bytes = bytemuck::bytes_of(&sink()).to_vec();
    info.write_field(&mut bytes, FieldId(1), &[0; 8]).unwrap();
    assert_eq!(decode::<KitchenSink>(&bytes).unwrap().origin, None);
    // A bare `NetId` field rejects zero.
    let bytes = [0u8; 8];
    assert!(decode::<BadTransient>(&bytes).is_err());
    assert!(BadTransient::INFO.write_field(&mut [1u8; 8], FieldId(1), &bytes).is_err());
}

/// Verification item, `docs/verification.md` section 2: on the pinned `bytemuck_derive` both
/// derives accept a data-carrying enum under `forbid(unsafe_code)`, given an explicit repr
/// and no padding in any variant.
#[test]
fn pinned_bytemuck_derives_accept_data_carrying_enums() {
    use lactose_reflect::bytemuck::{self, CheckedBitPattern, NoUninit};

    #[derive(Clone, Copy, PartialEq, Debug, NoUninit, CheckedBitPattern)]
    #[repr(u32)]
    enum Held {
        Nothing(u32) = 0,
        Item { slot: u32 } = 1,
    }

    let held = Held::Item { slot: 6 };
    let bytes = bytemuck::bytes_of(&held).to_vec();
    assert_eq!(bytes, [1, 0, 0, 0, 6, 0, 0, 0]);
    assert_eq!(bytemuck::checked::try_pod_read_unaligned::<Held>(&bytes), Ok(held));
    assert!(bytemuck::checked::try_pod_read_unaligned::<Held>(&[2, 0, 0, 0, 0, 0, 0, 0]).is_err());
    assert_eq!(bytemuck::checked::try_pod_read_unaligned::<Held>(&[0; 8]), Ok(Held::Nothing(0)));
}
