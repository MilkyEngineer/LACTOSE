use lactose_reflect::component;

/// Dereferences to the type the attribute recorded, so a check that let `&Wrapper` coerce to
/// `&u8` would accept it.
#[derive(Clone, Copy)]
pub struct Wrapper(u8);

impl core::ops::Deref for Wrapper {
    type Target = u8;

    fn deref(&self) -> &u8 {
        &self.0
    }
}

#[component(key = "ui::rewritten_field_deref", persistent)]
#[rewrite_macros::u8_to_wrapper]
pub struct Wrapped {
    #[field(id = 1)]
    pub x: u8,
}

fn main() {}
