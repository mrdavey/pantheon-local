//! Project a four-column portico onto a grid of Unicode braille cells.
type Point = (f32, f32, f32);

struct Canvas {
    width: usize,
    height: usize,
    angle: f32,
    dots: Vec<u8>,
}

impl Canvas {
    fn project(&self, (x, y, z): Point) -> (f32, f32) {
        let (sin, cos) = self.angle.sin_cos();
        let depth = z * cos - x * sin;
        let scale = (self.width as f32 * 2.0 / 26.0).min(self.height as f32 * 4.0 / 18.0);
        (
            self.width as f32 + (x * cos + z * sin) * scale,
            self.height as f32 * 2.0 + (y - depth * 0.22) * scale,
        )
    }

    fn stroke(&mut self, start: Point, end: Point) {
        let (x, y) = self.project(start);
        let (to_x, to_y) = self.project(end);
        let steps = ((to_x - x).abs().max((to_y - y).abs()).ceil() as usize).max(1);
        for step in 0..=steps {
            let fraction = step as f32 / steps as f32;
            let px = (x + (to_x - x) * fraction).round() as isize;
            let py = (y + (to_y - y) * fraction).round() as isize;
            if px < 0
                || py < 0
                || px >= (self.width * 2) as isize
                || py >= (self.height * 4) as isize
            {
                continue;
            }
            let (px, py) = (px as usize, py as usize);
            let bit = [[0, 1, 2, 6], [3, 4, 5, 7]][px % 2][py % 4];
            self.dots[py / 4 * self.width + px / 2] |= 1 << bit;
        }
    }

    fn outline(&mut self, points: &[Point]) {
        for pair in points.windows(2) {
            self.stroke(pair[0], pair[1]);
        }
        self.stroke(*points.last().unwrap(), points[0]);
    }
}

pub(super) fn portico(width: usize, height: usize, phase: f32) -> Vec<String> {
    let mut canvas = Canvas {
        width,
        height,
        angle: 0.18 + 0.65 * phase.sin(),
        dots: vec![0; width * height],
    };
    let (front, back) = (-2.6, 2.6);
    let near_side = if canvas.angle >= 0.0 { 1.0 } else { -1.0 };
    let eave = near_side * 9.4;
    canvas.outline(&[
        (0., -7.5, front),
        (0., -7.5, back),
        (eave, -3.5, back),
        (eave, -3.5, front),
    ]);
    canvas.outline(&[(-9.4, -3.5, front), (0., -7.5, front), (9.4, -3.5, front)]);
    canvas.outline(&[
        (-9.4, -3.5, front),
        (9.4, -3.5, front),
        (9.4, -2.3, front),
        (-9.4, -2.3, front),
    ]);
    canvas.outline(&[(-5.4, -4.25, front), (0., -6.6, front), (5.4, -4.25, front)]);
    canvas.outline(&[
        (eave, -3.5, back),
        (eave, -2.3, back),
        (eave, -2.3, front),
        (eave, -3.5, front),
    ]);
    let side_x = near_side * 7.2;
    canvas.stroke((side_x, -2.3, back), (side_x, 4.8, back));
    for x in [-7.2, -2.4, 2.4, 7.2] {
        canvas.outline(&[
            (x - 0.85, -2.3, front),
            (x + 0.85, -2.3, front),
            (x + 0.72, 4.8, front),
            (x - 0.72, 4.8, front),
        ]);
        canvas.stroke((x, -1.55, front), (x, 4.4, front));
        canvas.stroke((x - 1.25, -1.7, front), (x + 1.25, -1.7, front));
    }
    for (half, y) in [(9.2, 4.8), (10.2, 6.0)] {
        canvas.outline(&[
            (-half, y, front),
            (half, y, front),
            (half, y + 1.15, front),
            (-half, y + 1.15, front),
        ]);
        let edge = near_side * half;
        canvas.outline(&[
            (edge, y, front),
            (edge, y, back),
            (edge, y + 1.15, back),
            (edge, y + 1.15, front),
        ]);
    }
    canvas
        .dots
        .chunks(width)
        .map(|row| {
            row.iter()
                .map(|&bits| {
                    if bits == 0 {
                        ' '
                    } else {
                        char::from_u32(0x2800 + u32::from(bits)).unwrap()
                    }
                })
                .collect()
        })
        .collect()
}
