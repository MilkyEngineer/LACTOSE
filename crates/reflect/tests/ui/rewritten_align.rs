use lactose_reflect::component;

#[component(key = "ui::rewritten_align", persistent)]
#[rewrite_macros::align16]
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
