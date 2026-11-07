//! Selected Microsoft Fluent System Icons, MIT. See assets/fluent/provenance.json.
use crate::data::Result;
use windows::Win32::Graphics::Direct2D::{Common::*, *};
use windows_numerics::Vector2;
pub unsafe fn seek(factory: &ID2D1Factory) -> Result<ID2D1PathGeometry> {
    unsafe {
        let geometry = factory.CreatePathGeometry()?;
        let sink = geometry.Open()?;
        let p = |x, y| Vector2 { X: x, Y: y };
        sink.BeginFigure(p(12., 4.5), D2D1_FIGURE_BEGIN_FILLED);
        sink.AddBezier(&D2D1_BEZIER_SEGMENT {
            point1: p(16.1421, 4.5),
            point2: p(19.5, 7.85786),
            point3: p(19.5, 12.),
        });
        sink.AddBezier(&D2D1_BEZIER_SEGMENT {
            point1: p(19.5, 16.1421),
            point2: p(16.1421, 19.5),
            point3: p(12., 19.5),
        });
        sink.AddBezier(&D2D1_BEZIER_SEGMENT {
            point1: p(7.85786, 19.5),
            point2: p(4.5, 16.1421),
            point3: p(4.5, 12.),
        });
        sink.AddBezier(&D2D1_BEZIER_SEGMENT {
            point1: p(4.5, 11.6236),
            point2: p(4.52772, 11.2538),
            point3: p(4.58123, 10.8923),
        });
        sink.AddBezier(&D2D1_BEZIER_SEGMENT {
            point1: p(4.64845, 10.4382),
            point2: p(4.31609, 10.),
            point3: p(3.85708, 10.),
        });
        sink.AddBezier(&D2D1_BEZIER_SEGMENT {
            point1: p(3.48623, 10.),
            point2: p(3.161, 10.2562),
            point3: p(3.10471, 10.6228),
        });
        sink.AddBezier(&D2D1_BEZIER_SEGMENT {
            point1: p(3.03576, 11.0718),
            point2: p(3., 11.5317),
            point3: p(3., 12.),
        });
        sink.AddBezier(&D2D1_BEZIER_SEGMENT {
            point1: p(3., 16.9706),
            point2: p(7.02944, 21.),
            point3: p(12., 21.),
        });
        sink.AddBezier(&D2D1_BEZIER_SEGMENT {
            point1: p(16.9706, 21.),
            point2: p(21., 16.9706),
            point3: p(21., 12.),
        });
        sink.AddBezier(&D2D1_BEZIER_SEGMENT {
            point1: p(21., 7.02944),
            point2: p(16.9706, 3.),
            point3: p(12., 3.),
        });
        sink.AddBezier(&D2D1_BEZIER_SEGMENT {
            point1: p(9.69494, 3.),
            point2: p(7.59227, 3.86656),
            point3: p(6., 5.29168),
        });
        sink.AddLine(p(6., 4.25));
        sink.AddBezier(&D2D1_BEZIER_SEGMENT {
            point1: p(6., 3.83579),
            point2: p(5.66421, 3.5),
            point3: p(5.25, 3.5),
        });
        sink.AddBezier(&D2D1_BEZIER_SEGMENT {
            point1: p(4.83579, 3.5),
            point2: p(4.5, 3.83579),
            point3: p(4.5, 4.25),
        });
        sink.AddLine(p(4.5, 7.25));
        sink.AddBezier(&D2D1_BEZIER_SEGMENT {
            point1: p(4.5, 7.66421),
            point2: p(4.83579, 8.),
            point3: p(5.25, 8.),
        });
        sink.AddLine(p(8.25, 8.));
        sink.AddBezier(&D2D1_BEZIER_SEGMENT {
            point1: p(8.66421, 8.),
            point2: p(9., 7.66421),
            point3: p(9., 7.25),
        });
        sink.AddBezier(&D2D1_BEZIER_SEGMENT {
            point1: p(9., 6.83579),
            point2: p(8.66421, 6.5),
            point3: p(8.25, 6.5),
        });
        sink.AddLine(p(6.90093, 6.5));
        sink.AddBezier(&D2D1_BEZIER_SEGMENT {
            point1: p(8.23907, 5.25883),
            point2: p(10.0309, 4.5),
            point3: p(12., 4.5),
        });
        sink.EndFigure(D2D1_FIGURE_END_CLOSED);
        sink.Close()?;
        Ok(geometry)
    }
}
pub unsafe fn check(factory: &ID2D1Factory) -> Result<ID2D1PathGeometry> {
    unsafe {
        let geometry = factory.CreatePathGeometry()?;
        let sink = geometry.Open()?;
        let p = |x, y| Vector2 { X: x, Y: y };
        sink.BeginFigure(p(13.8639, 3.65511), D2D1_FIGURE_BEGIN_FILLED);
        sink.AddBezier(&D2D1_BEZIER_SEGMENT {
            point1: p(14.0533, 3.85606),
            point2: p(14.0439, 4.17251),
            point3: p(13.8429, 4.36191),
        });
        sink.AddLine(p(5.91309, 11.8358));
        sink.AddBezier(&D2D1_BEZIER_SEGMENT {
            point1: p(5.67573, 12.0595),
            point2: p(5.30311, 12.0526),
            point3: p(5.07417, 11.8203),
        });
        sink.AddLine(p(2.39384, 9.09995));
        sink.AddBezier(&D2D1_BEZIER_SEGMENT {
            point1: p(2.20003, 8.90325),
            point2: p(2.20237, 8.58667),
            point3: p(2.39907, 8.39286),
        });
        sink.AddBezier(&D2D1_BEZIER_SEGMENT {
            point1: p(2.59578, 8.19905),
            point2: p(2.91235, 8.2014),
            point3: p(3.10616, 8.3981),
        });
        sink.AddLine(p(5.51192, 10.8398));
        sink.AddLine(p(13.1571, 3.63419));
        sink.AddBezier(&D2D1_BEZIER_SEGMENT {
            point1: p(13.358, 3.44479),
            point2: p(13.6745, 3.45416),
            point3: p(13.8639, 3.65511),
        });
        sink.EndFigure(D2D1_FIGURE_END_CLOSED);
        sink.Close()?;
        Ok(geometry)
    }
}
