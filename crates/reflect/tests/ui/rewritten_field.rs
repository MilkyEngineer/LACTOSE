use lactose_reflect::component;

#[component(key = "ui::rewritten_field", persistent)]
#[rewrite_macros::u8_to_bool]
pub struct Flag {
    #[field(id = 1)]
    pub x: u8,
}

fn main() {}
