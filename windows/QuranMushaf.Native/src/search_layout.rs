//! Shared geometry for the native Search controls and their painted surface.
#[derive(Clone, Copy)]
pub struct SearchLayout {
    pub title: f32,
    pub subtitle: f32,
    pub query_frame: f32,
    pub tabs: f32,
    pub tab_height: f32,
    pub section_label: f32,
    pub section_caption: bool,
    pub filter_frame: f32,
    pub browse_caption: bool,
    pub browse_results: f32,
}
impl SearchLayout {
    pub fn new(height: f32) -> Self {
        if height < 400. {
            Self {
                title: 54.,
                subtitle: 80.,
                query_frame: 112.,
                tabs: 172.,
                tab_height: 34.,
                section_label: 216.,
                filter_frame: 216.,
                section_caption: false,
                browse_caption: false,
                browse_results: 276.,
            }
        } else if height < 520. {
            Self {
                title: 58.,
                subtitle: 86.,
                query_frame: 120.,
                tabs: 184.,
                tab_height: 34.,
                section_label: 230.,
                filter_frame: 260.,
                section_caption: true,
                browse_caption: false,
                browse_results: 320.,
            }
        } else {
            Self {
                title: 64.,
                subtitle: 96.,
                query_frame: 130.,
                tabs: 226.,
                tab_height: 40.,
                section_label: 280.,
                filter_frame: 314.,
                section_caption: true,
                browse_caption: true,
                browse_results: 374.,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn short_search_keeps_inputs_and_a_result_inside_the_screen() {
        for height in [360., 400., 432., 500., 540., 600., 1080.] {
            let layout = SearchLayout::new(height);
            assert!(layout.title + 24. <= layout.subtitle);
            assert!(layout.subtitle + 24. <= layout.query_frame);
            assert!(layout.query_frame + 48. < layout.tabs);
            if layout.section_caption {
                assert!(layout.tabs + layout.tab_height <= layout.section_label);
                assert!(layout.section_label + 24. <= layout.filter_frame);
            } else {
                assert!(layout.tabs + layout.tab_height + 10. <= layout.filter_frame);
            }
            assert!(layout.filter_frame + 48. < layout.browse_results);
            assert!(layout.browse_results + 42. <= height - 32.);
        }
    }
}
