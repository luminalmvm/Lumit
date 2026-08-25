use flutter_rust_bridge::frb;

#[frb(opaque)]
pub struct Expressions {}

impl Expressions {
    pub fn get_expressions_metadata() -> String {
        lumit_core::expression::get_api_metadata()
    }
}
