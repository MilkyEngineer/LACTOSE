use lactose_reflect::component;

#[component(key = "ui::raw_repr", persistent)]
#[r#repr(align(16))]
pub struct Aligned {
    #[field(id = 1)]
    pub a: u32,
    #[field(id = 2)]
    pub b: u32,
    #[field(id = 3)]
    pub c: u32,
    #[field(id = 4)]
    pub d: u32,
}

fn main() {}
