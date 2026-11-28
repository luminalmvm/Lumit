use super::Layer;

impl Layer {
    /// How fast the layer's source runs at layer time `lt`, 1.0 being 100%.
    /// An expression on Retime is read as [`Layer::source_time_at`] reads it,
    /// with the comp's time. Read without, it holds still, and Flow never
    /// engages.
    pub fn speed_at(&self, lt: f64) -> f64 {
        match &self.retime {
            Some(r) if matches!(r.animation, crate::anim::Animation::Expression(_)) => {
                const H: f64 = 1e-3;
                (self.source_time_at(lt + H) - self.source_time_at(lt - H)) / (2.0 * H)
            }
            retime => crate::retime::property_speed_at(retime.as_ref(), lt),
        }
    }
}
