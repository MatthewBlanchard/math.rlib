use super::{Bezier, Evaluate, Piecewise, Vector, GlyphBuilder};
use super::consts::{SMALL_DISTANCE};
use super::piecewise::glif::PointData;
use glifparser::{Glif, Outline};

#[derive(Debug, Clone)]
pub struct VWSContour {
    pub id: usize,
    pub handles: Vec<VWSHandle>
}

#[derive(Debug, Clone, Copy)]
pub enum InterpolationType {
    Linear,
    Null
}

#[derive(Debug, Clone, Copy)]
pub struct VWSHandle {
    pub left_offset: f64,
    pub right_offset: f64,
    pub interpolation: InterpolationType
}

pub enum JoinType {
    Bevel,
    Miter,
    Round
}

pub enum CapType {
    Round,
    Square,
    Custom
}

pub struct VWSSettings {
    pub join_type: JoinType,
    pub cap_type_start: CapType,
    pub cap_type_end: CapType,
    pub cap_custom_start: Option<Glif<Option<PointData>>>,
    pub cap_custom_end: Option<Glif<Option<PointData>>>,
}

// takes a vector of beziers and fills in discontinuities with joins
fn fix_path(in_path: GlyphBuilder, closed: bool, settings: &VWSSettings) -> GlyphBuilder
{
    let mut out = GlyphBuilder::new();

    let join_to = match &settings.join_type {
        JoinType::Bevel => GlyphBuilder::bevel_to,
        JoinType::Round => GlyphBuilder::arc_to,
        JoinType::Miter => GlyphBuilder::miter_to
    };

    let mut path_iter = in_path.beziers.iter().peekable();
    
    while let Some(bezier) = path_iter.next() {
        if let Some(next_bezier) = path_iter.peek()
        {
            let next_start = next_bezier.start_point();
            let last_end = bezier.end_point();
            if !last_end.is_near(next_start, SMALL_DISTANCE/2.)
            {
                // the end of our last curve doesn't match up with the start of our next so we need to
                // deal with the discontinuity be creating a join
                let tangent1 = bezier.tangent_at(1.).normalize(); 
                let tangent2 = -next_bezier.tangent_at(0.).normalize();
                let discontinuity_vec = next_start - last_end;
                let on_outside = Vector::dot(tangent2, discontinuity_vec) >= 0.;
                
                if !on_outside {
                    //TODO: implement more complicated joins
                    out.bezier_to(bezier.clone());
                    join_to(&mut out, next_start, tangent1, tangent2);
                }
                else
                {
                    // we're inside so we default to a bevel
                    out.bezier_to(bezier.clone());
                    out.line_to(next_start);
                }
            }
            else
            {
                out.bezier_to(bezier.clone());
            }
        }
        else if closed
        {
            // our path is closed and if there's not a next point we need to make sure that our current
            // and last curve matches up with the first one

            let first_bez = in_path.beziers.first().unwrap();
            let first_point = first_bez.start_point();
            let last_end = bezier.end_point();

            if !last_end.is_near(first_point, SMALL_DISTANCE)
            {
                let tangent1 = bezier.tangent_at(1.).normalize(); 
                let tangent2 = -first_bez.tangent_at(0.).normalize();
                let discontinuity_vec = first_point - last_end;
                let on_outside = Vector::dot(tangent2, discontinuity_vec) >= 0.;

                if !on_outside {
                    out.bezier_to(bezier.clone());
                    join_to(&mut out, first_point, tangent1, tangent2);
                }
                else
                {
                    out.bezier_to(bezier.clone());
                    out.line_to(first_point);
                }
            }
            else
            {
                out.bezier_to(bezier.clone());
            }
        }
        else
        {
            out.bezier_to(bezier.clone());
        }
    }

    return out;
}

pub fn variable_width_stroke(in_pw: &Piecewise<Bezier>, stroke_handles: &Vec<VWSHandle>, settings: &VWSSettings) -> Piecewise<Piecewise<Bezier>> {
    let closed = in_pw.is_closed();

    // check if our input path is closed
    // We're gonna keep track of a left line and a right line.
    let mut left_line = GlyphBuilder::new();
    let mut right_line = GlyphBuilder::new();

    let iter = in_pw.segs.iter().enumerate();
    for (i, bezier) in iter {
        let cur_handle = &stroke_handles[i];
        let next_handle = &stroke_handles[i+1];

        let left_start = cur_handle.left_offset;
        let right_start = cur_handle.right_offset;

        let left_end = match cur_handle.interpolation {
            InterpolationType::Null => left_start,
            _ => next_handle.left_offset
        };

        
        let right_end = match cur_handle.interpolation {
            InterpolationType::Null => right_start,
            _ => next_handle.right_offset
        };

        let left_offset = flo_curves::bezier::offset(bezier, -left_start, -left_end);
        left_line.append_vec(left_offset);

        let right_offset = flo_curves::bezier::offset(bezier, right_start, right_end);
        right_line.append_vec(right_offset);
    }
     
    right_line.beziers.reverse();
    right_line = GlyphBuilder {
        beziers:    right_line.beziers.iter()
                    .map(|bez| bez.clone().reverse())
                    .collect()
    };

    right_line = fix_path(right_line, closed, &settings);
    left_line = fix_path(left_line, closed, &settings);

    if in_pw.is_closed() {
        let mut out = Vec::new();

        let left_pw = Piecewise::new(left_line.beziers, None);
        let right_pw = Piecewise::new(right_line.beziers, None);

        out.push(left_pw);
        out.push(right_pw);
        
        return Piecewise::new(out, None);
    }
    else
    {
        // path is not closed we need to cap the ends
        let mut out_builder = left_line;


        let from = out_builder.beziers.last().unwrap().clone();
        let to = right_line.beziers.first().unwrap().clone();
        
        // used for round joins
        let tangent1 = from.tangent_at(1.).normalize(); 
        let tangent2 = -to.tangent_at(0.).normalize();

        match &settings.cap_type_end {
            CapType::Round => out_builder.arc_to(to.start_point(), tangent1, tangent2),
            CapType::Square => out_builder.line_to(to.start_point()),
            CapType::Custom => out_builder.cap_to(to.start_point(), settings.cap_custom_end.as_ref().unwrap())
        }

        // append the right line to the left now that we've connected them
        out_builder.append(right_line);

        // we need to close the beginning now 
        let from = out_builder.beziers.last().unwrap().clone();
        let to = out_builder.beziers.first().unwrap().clone();

        let tangent1 = from.tangent_at(1.).normalize(); 
        let tangent2 = to.tangent_at(0.).normalize();

        match &settings.cap_type_start {
            CapType::Round => out_builder.arc_to(to.end_point(), tangent1, tangent2),
            CapType::Square => out_builder.line_to(to.end_point()),
            CapType::Custom => {}//currently unhandled
        }

        let inner = Piecewise::new(out_builder.beziers, None);
        return Piecewise::new(vec![inner], None);
    } 

}

pub fn variable_width_stroke_glif<T>(path: &Glif<T>, settings: VWSSettings) -> Glif<Option<PointData>>
{
    // convert our path and pattern to piecewise collections of beziers
    let piece_path = Piecewise::from(path.outline.as_ref().unwrap());
    let mut output_outline: Outline<Option<PointData>> = Vec::new();

    let handles = parse_vws_lib(path);

    if handles.is_none() {
        panic!("No vws contours found in input!")
    }
    
    let handles = handles.expect("Input glyph has no lib node!");

    let iter = piece_path.segs.iter().enumerate();
    for (i, pwpath_contour) in iter {
        let vws_contour = find_vws_contour(i, &handles.0);
        
        if let Some(contour) = vws_contour {
            let results = variable_width_stroke(&pwpath_contour, &contour.handles, &settings);
            for result_contour in results.segs {
                output_outline.push(result_contour.to_contour());
            }
        }
        else
        {
            output_outline.push(pwpath_contour.to_contour());
        }
    }
    
    return Glif {
        outline: Some(output_outline),
        order: path.order, // default when only corners
        anchors: path.anchors.clone(),
        width: path.width,
        unicode: path.unicode,
        name: path.name.clone(),
        format: 2,
        lib: Some(handles.1)
    };
}

pub fn find_vws_contour(id: usize, vws_outline: &Vec<VWSContour>) -> Option<&VWSContour>
{
    for contour in vws_outline {
        if contour.id == id {
            return Some(contour);
        }
    }

    return None;
}

pub fn parse_vws_lib<T>(input: &Glif<T>) -> Option<(Vec<VWSContour>, xmltree::Element)>
{
    if let Some(lib) = input.lib.as_ref() {
        let mut lib = lib.clone();
        let mut vws_outline = Vec::new();

        while let Some(mut vws) = lib.take_child("vws") {
            let name = vws
                .attributes
                .get("id")
                .expect("VWSContour must have an id");

            let mut vws_handles = VWSContour {
                id: name.parse().unwrap(),
                handles: Vec::new()
            };

            while let Some(vws_handle) = vws.take_child("handle") {
                let left: f64 = vws_handle
                    .attributes
                    .get("left")
                    .expect("VWSHandle missing left")
                    .parse()
                    .expect("VWSHandle not float.");

                let right: f64 = vws_handle
                    .attributes
                    .get("right")
                    .expect("VWSHandle missing right")
                    .parse()
                    .expect("VWSHandle not float.");

                let interpolation_string: &String = vws_handle
                    .attributes
                    .get("interpolation")
                    .expect("VWSHandle missing interpolation type");

                let interpolation = match interpolation_string.as_str() {
                    "linear" => InterpolationType::Linear,
                    _ => InterpolationType::Null
                };

                vws_handles.handles.push(VWSHandle{
                    left_offset: left,
                    right_offset: right,
                    interpolation: interpolation
                });
            }

            vws_outline.push(vws_handles);
        }

        if vws_outline.len() > 0 {
            return Some((vws_outline, lib));
        }
    }

    return None;
}

pub fn generate_vws_lib(vwscontours:  &Vec<VWSContour>) -> Option<xmltree::Element>
{
    if vwscontours.len() == 0 { return None }
    let mut lib_node = xmltree::Element::new("lib");

    for vwcontour in vwscontours {
        let mut vws_node = xmltree::Element::new("vws");
         vws_node.attributes.insert("id".to_owned(), vwcontour.id.to_string());

        for handle in &vwcontour.handles {
            let mut handle_node = xmltree::Element::new("handle");
            handle_node.attributes.insert("left".to_owned(), handle.left_offset.to_string());
            handle_node.attributes.insert("right".to_owned(), handle.right_offset.to_string());

            match handle.interpolation {
                InterpolationType::Linear => {handle_node.attributes.insert("interpolation".to_owned(), "linear".to_owned());},
                InterpolationType::Null => {handle_node.attributes.insert("interpolation".to_owned(), "none".to_owned());}
            }
            
            vws_node.children.push(xmltree::XMLNode::Element(handle_node));
        }

        lib_node.children.push(xmltree::XMLNode::Element(vws_node));
    }

    return Some(lib_node);
}