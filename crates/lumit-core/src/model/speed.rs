use super::Layer;

impl Layer {
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
