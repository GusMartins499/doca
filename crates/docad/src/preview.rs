//! A window, shrunk to a picture the dock can show over its icon (#18, P3).
//!
//! The X server hands over a window's pixels as they are, at full size: a
//! 4K window is 33 MB. What leaves here is the picture the hover shows,
//! a couple of hundred pixels across, so the size is brought down on this
//! side of the bus and the bus never carries the full frame.

/// The most a caller is given, whatever it asks for. A preview is a glance,
/// and a caller asking for the whole screen is asking for a screenshot.
pub const LARGEST: u32 = 640;

/// A window's picture: width, height, and four bytes a pixel, RGBA, rows
/// packed with no padding — what `gdk_pixbuf` takes as it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shot {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// The size a `width` × `height` window is drawn at inside `room`, in
/// proportion and never larger than it is: a small dialog stays small
/// rather than being blown up into a blur.
pub fn fit(width: u32, height: u32, room: (u32, u32)) -> (u32, u32) {
    if width == 0 || height == 0 || room.0 == 0 || room.1 == 0 {
        return (0, 0);
    }
    let scale = (room.0 as f64 / width as f64)
        .min(room.1 as f64 / height as f64)
        .min(1.0);
    let across = ((width as f64 * scale).round() as u32).max(1);
    let down = ((height as f64 * scale).round() as u32).max(1);
    (across, down)
}

/// Shrink a frame the X server sent to `to` pixels.
///
/// `bgrx` is a ZPixmap at 32 bits a pixel in the server's LSB-first order,
/// which is blue, green, red and a fourth byte. The fourth byte is not read:
/// on a 24-bit window it is padding, and on a 32-bit one it is alpha already
/// multiplied into the colours — so the picture is the window over black,
/// which is how a translucent terminal looks with nothing behind it.
///
/// Each pixel of the picture is the average of the block of the frame it
/// covers. Taking one pixel of each block instead is faster and turns text
/// into noise, and text is most of what a window is.
pub fn shrink(bgrx: &[u8], stride: usize, size: (u32, u32), to: (u32, u32)) -> Shot {
    let (width, height) = (size.0 as usize, size.1 as usize);
    let (across, down) = (to.0 as usize, to.1 as usize);
    let mut rgba = vec![0u8; across * down * 4];
    for out_y in 0..down {
        let top = out_y * height / down;
        let bottom = ((out_y + 1) * height / down).max(top + 1).min(height);
        for out_x in 0..across {
            let left = out_x * width / across;
            let right = ((out_x + 1) * width / across).max(left + 1).min(width);
            let mut sum = [0u64; 3];
            for y in top..bottom {
                let row = y * stride;
                for x in left..right {
                    let at = row + x * 4;
                    sum[0] += bgrx[at + 2] as u64;
                    sum[1] += bgrx[at + 1] as u64;
                    sum[2] += bgrx[at] as u64;
                }
            }
            let count = ((bottom - top) * (right - left)) as u64;
            let at = (out_y * across + out_x) * 4;
            rgba[at] = (sum[0] / count) as u8;
            rgba[at + 1] = (sum[1] / count) as u8;
            rgba[at + 2] = (sum[2] / count) as u8;
            rgba[at + 3] = 255;
        }
    }
    Shot {
        width: to.0,
        height: to.1,
        rgba,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wide_window_fills_the_width_and_keeps_its_proportion() {
        assert_eq!(fit(1920, 1080, (240, 160)), (240, 135));
    }

    #[test]
    fn a_tall_window_fills_the_height_and_keeps_its_proportion() {
        assert_eq!(fit(600, 1200, (240, 160)), (80, 160));
    }

    #[test]
    fn a_window_smaller_than_the_room_is_not_blown_up() {
        assert_eq!(fit(120, 80, (240, 160)), (120, 80));
    }

    #[test]
    fn a_sliver_of_a_window_is_still_a_pixel_and_not_nothing() {
        assert_eq!(fit(4000, 2, (240, 160)), (240, 1));
    }

    #[test]
    fn nothing_to_draw_or_nowhere_to_draw_it_is_no_picture() {
        assert_eq!(fit(0, 1080, (240, 160)), (0, 0));
        assert_eq!(fit(1920, 1080, (0, 160)), (0, 0));
    }

    /// A frame in the server's order: blue, green, red, then a byte nobody reads.
    fn frame(pixels: &[[u8; 3]], width: usize, padding: usize) -> (Vec<u8>, usize) {
        let stride = width * 4 + padding;
        let mut bytes = Vec::new();
        for row in pixels.chunks(width) {
            for [r, g, b] in row {
                bytes.extend_from_slice(&[*b, *g, *r, 7]);
            }
            bytes.extend(std::iter::repeat(0xee).take(padding));
        }
        (bytes, stride)
    }

    #[test]
    fn the_servers_blue_green_red_comes_out_red_green_blue_and_opaque() {
        let (bytes, stride) = frame(&[[10, 20, 30]], 1, 0);

        let shot = shrink(&bytes, stride, (1, 1), (1, 1));

        assert_eq!(shot.rgba, vec![10, 20, 30, 255]);
    }

    #[test]
    fn a_shrunk_pixel_is_the_average_of_what_it_covers_and_not_one_of_them() {
        let white = [255, 255, 255];
        let black = [0, 0, 0];
        let (bytes, stride) = frame(&[white, black, black, white], 2, 0);

        let shot = shrink(&bytes, stride, (2, 2), (1, 1));

        assert_eq!(&shot.rgba[..3], &[127, 127, 127]);
    }

    #[test]
    fn the_padding_at_the_end_of_a_row_is_never_read_as_a_pixel() {
        let red = [200, 0, 0];
        let (bytes, stride) = frame(&[red, red, red, red], 2, 8);

        let shot = shrink(&bytes, stride, (2, 2), (2, 2));

        for pixel in shot.rgba.chunks(4) {
            assert_eq!(pixel, &[200, 0, 0, 255]);
        }
    }

    #[test]
    fn a_shot_is_four_bytes_a_pixel_with_nothing_between_the_rows() {
        let (bytes, stride) = frame(&vec![[1, 2, 3]; 7 * 5], 7, 4);

        let shot = shrink(&bytes, stride, (7, 5), (3, 2));

        assert_eq!((shot.width, shot.height), (3, 2));
        assert_eq!(shot.rgba.len(), 3 * 2 * 4);
    }
}
