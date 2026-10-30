#[cfg(test)]
use super::*;
#[cfg(test)]
use rgb::RGBA8;

    fn create_test_image(width: u32, height: u32, pixels: Vec<RGBA8>) -> ImageData {
        ImageData {
            width,
            height,
            pixels,
        }
    }

    fn create_solid_color_image(width: u32, height: u32, color: RGBA8) -> ImageData {
        let pixels = vec![color; (width * height) as usize];
        ImageData {
            width,
            height,
            pixels,
        }
    }

    fn create_gradient_image(width: u32, height: u32) -> ImageData {
        let mut pixels = Vec::with_capacity((width * height) as usize);
        for y in 0..height {
            for x in 0..width {
                let r = (x * 255 / width.max(1)) as u8;
                let g = (y * 255 / height.max(1)) as u8;
                let b = 128;
                pixels.push(RGBA8::new(r, g, b, 255));
            }
        }
        ImageData {
            width,
            height,
            pixels,
        }
    }

    #[test]
    fn test_image_data_creation() {
        let img = create_test_image(10, 10, vec![RGBA8::new(255, 0, 0, 255); 100]);
        assert_eq!(img.width, 10);
        assert_eq!(img.height, 10);
        assert_eq!(img.pixels.len(), 100);
    }

    #[test]
    fn test_quantize_colors_reduces_to_exact_count() {
        let img = create_gradient_image(50, 50);
        let num_colors = 8;

        let result = quantize_colors(&img, num_colors);
        assert!(result.is_ok());

        let quantized = result.unwrap();
        assert_eq!(quantized.width, 50);
        assert_eq!(quantized.height, 50);

        // Count unique colors
        let unique_colors: std::collections::HashSet<_> =
            quantized.pixels.iter().map(|p| (p.r, p.g, p.b)).collect();

        // Should have at most num_colors unique colors
        assert!(unique_colors.len() <= num_colors);
    }

    #[test]
    fn test_quantize_colors_with_single_color() {
        let red = RGBA8::new(255, 0, 0, 255);
        let img = create_solid_color_image(10, 10, red);

        let result = quantize_colors(&img, 16);
        assert!(result.is_ok());

        let quantized = result.unwrap();
        for pixel in quantized.pixels {
            assert_eq!(pixel.r, 255);
            assert_eq!(pixel.g, 0);
            assert_eq!(pixel.b, 0);
        }
    }

    #[test]
    fn test_quantize_colors_preserves_dimensions() {
        let img = create_gradient_image(100, 50);
        let result = quantize_colors(&img, 16);

        assert!(result.is_ok());
        let quantized = result.unwrap();
        assert_eq!(quantized.width, img.width);
        assert_eq!(quantized.height, img.height);
    }

    #[test]
    fn test_quantize_colors_with_zero_colors() {
        let img = create_solid_color_image(10, 10, RGBA8::new(128, 128, 128, 255));
        let result = quantize_colors(&img, 0);

        // With 0 colors, the function returns an error (unwrap on empty palette)
        assert!(result.is_err());
    }

    #[test]
    fn test_quantize_colors_preserves_alpha() {
        let mut pixels = vec![RGBA8::new(255, 0, 0, 255); 50];
        pixels.extend(vec![RGBA8::new(0, 255, 0, 128); 50]);

        let img = create_test_image(10, 10, pixels);
        let result = quantize_colors(&img, 2);

        assert!(result.is_ok());
        let quantized = result.unwrap();

        // Check that alpha values are preserved
        assert_eq!(quantized.pixels[0].a, 255);
        assert_eq!(quantized.pixels[50].a, 128);
    }

    #[test]
    fn test_quantize_colors_with_two_distinct_colors() {
        let mut pixels = vec![RGBA8::new(255, 0, 0, 255); 50];
        pixels.extend(vec![RGBA8::new(0, 0, 255, 255); 50]);

        let img = create_test_image(10, 10, pixels);
        let result = quantize_colors(&img, 2);

        assert!(result.is_ok());
        let quantized = result.unwrap();

        // Count how many of each color we have
        let mut red_count = 0;
        let mut blue_count = 0;

        for pixel in &quantized.pixels {
            if pixel.r > 200 && pixel.b < 50 {
                red_count += 1;
            } else if pixel.b > 200 && pixel.r < 50 {
                blue_count += 1;
            }
        }

        // Should preserve the two color regions
        assert!(red_count > 0);
        assert!(blue_count > 0);
    }

    #[test]
    fn test_median_cut_with_grayscale() {
        let mut pixels = Vec::new();
        for i in 0..256 {
            let val = i as u8;
            pixels.push(RGBA8::new(val, val, val, 255));
        }

        let img = create_test_image(16, 16, pixels);
        let result = quantize_colors(&img, 4);

        assert!(result.is_ok());
        let quantized = result.unwrap();

        // Check that we have reduced the color space
        let unique_colors: std::collections::HashSet<_> =
            quantized.pixels.iter().map(|p| (p.r, p.g, p.b)).collect();

        assert!(unique_colors.len() <= 4);
    }

    #[test]
    fn test_box_max_range() {
        let colors = vec![(0, 0, 0), (255, 255, 255), (128, 128, 128)];
        let range = crate::image_processor::box_max_range(&colors);
        assert_eq!(range, 255);
    }

    #[test]
    fn test_box_average() {
        let colors = vec![(0, 0, 0), (255, 255, 255)];
        let avg = crate::image_processor::box_average(&colors);
        assert_eq!(avg.r, 127);
        assert_eq!(avg.g, 127);
        assert_eq!(avg.b, 127);
        assert_eq!(avg.a, 255);
    }

    #[test]
    fn test_box_average_empty() {
        let colors: Vec<(u8, u8, u8)> = vec![];
        let avg = crate::image_processor::box_average(&colors);
        assert_eq!(avg.r, 0);
        assert_eq!(avg.g, 0);
        assert_eq!(avg.b, 0);
        assert_eq!(avg.a, 255);
    }

    #[test]
    fn test_split_box_red_channel() {
        let colors = vec![
            (0, 128, 128),
            (255, 128, 128),
            (100, 128, 128),
            (200, 128, 128),
        ];

        let (left, right): (Vec<(u8, u8, u8)>, Vec<(u8, u8, u8)>) = crate::image_processor::split_box(colors);
        assert!(!left.is_empty());
        assert!(!right.is_empty());
        assert_eq!(left.len() + right.len(), 4);
    }

    #[test]
    fn test_resize_if_needed_no_resize() {
        let img = create_solid_color_image(100, 100, RGBA8::new(128, 128, 128, 255));
        let result = resize_if_needed(img, 4096);
        assert_eq!(result.width, 100);
        assert_eq!(result.height, 100);
    }

    #[test]
    fn test_resize_if_needed_downscale() {
        let img = create_solid_color_image(200, 100, RGBA8::new(255, 0, 0, 255));
        let result = resize_if_needed(img, 50);
        // 200x100 scaled to fit 50x50 → scale = 50/200 = 0.25 → 50x25
        assert_eq!(result.width, 50);
        assert_eq!(result.height, 25);
        assert_eq!(result.pixels.len(), 50 * 25);
    }

    #[test]
    fn test_resize_if_needed_preserves_aspect_ratio() {
        let img = create_solid_color_image(300, 600, RGBA8::new(0, 255, 0, 255));
        let result = resize_if_needed(img, 100);
        // 300x600 → scale = 100/600 = 0.1667 → 50x100
        assert_eq!(result.width, 50);
        assert_eq!(result.height, 100);
    }

    #[test]
    fn test_resize_if_needed_exact_boundary() {
        let img = create_solid_color_image(4096, 4096, RGBA8::new(0, 0, 0, 255));
        let result = resize_if_needed(img, 4096);
        assert_eq!(result.width, 4096);
        assert_eq!(result.height, 4096);
    }

    #[test]
    fn test_cmyk_to_rgb_white() {
        // CMYK(0,0,0,0) → RGB(255,255,255)
        let (r, g, b) = cmyk_to_rgb(0, 0, 0, 0);
        assert_eq!(r, 255);
        assert_eq!(g, 255);
        assert_eq!(b, 255);
    }

    #[test]
    fn test_cmyk_to_rgb_black() {
        // CMYK(0,0,0,255) → RGB(0,0,0)
        let (r, g, b) = cmyk_to_rgb(0, 0, 0, 255);
        assert_eq!(r, 0);
        assert_eq!(g, 0);
        assert_eq!(b, 0);
    }

    #[test]
    fn test_cmyk_to_rgb_cyan() {
        // CMYK(255,0,0,0) → RGB(0,255,255) (cyan ink = no red)
        let (r, g, b) = cmyk_to_rgb(255, 0, 0, 0);
        assert_eq!(r, 0);
        assert_eq!(g, 255);
        assert_eq!(b, 255);
    }

    #[test]
    fn test_cmyk_to_rgb_magenta() {
        // CMYK(0,255,0,0) → RGB(255,0,255) (magenta ink = no green)
        let (r, g, b) = cmyk_to_rgb(0, 255, 0, 0);
        assert_eq!(r, 255);
        assert_eq!(g, 0);
        assert_eq!(b, 255);
    }

    #[test]
    fn test_cmyk_to_rgb_yellow() {
        // CMYK(0,0,255,0) → RGB(255,255,0) (yellow ink = no blue)
        let (r, g, b) = cmyk_to_rgb(0, 0, 255, 0);
        assert_eq!(r, 255);
        assert_eq!(g, 255);
        assert_eq!(b, 0);
    }

    #[test]
    fn test_cmyk_to_rgb_red() {
        // CMYK(0,255,255,0) → RGB(255,0,0) (no cyan, full magenta + yellow = red)
        let (r, g, b) = cmyk_to_rgb(0, 255, 255, 0);
        assert!(r > 250, "Red should be high: {}", r);
        assert!(g < 5, "Green should be near zero: {}", g);
        assert!(b < 5, "Blue should be near zero: {}", b);
    }

    #[test]
    fn test_cmyk_to_rgb_with_black_component() {
        // CMYK(0,255,255,128) → darker red
        let (r, g, b) = cmyk_to_rgb(0, 255, 255, 128);
        assert!(r > 100 && r < 150, "Red should be mid-range with K component");
        assert!(g < 5);
        assert!(b < 5);
    }

    #[test]
    fn test_image_data_from_cmyk() {
        let cmyk = vec![
            (0, 0, 0, 0),    // white
            (255, 0, 0, 0),  // cyan (no red)
            (0, 255, 0, 0),  // magenta (no green)
            (0, 0, 255, 0),  // yellow (no blue)
        ];
        let img = image_data_from_cmyk(&cmyk, 2, 2);
        assert_eq!(img.width, 2);
        assert_eq!(img.height, 2);
        assert_eq!(img.pixels[0], RGBA8::new(255, 255, 255, 255)); // white
        assert_eq!(img.pixels[1], RGBA8::new(0, 255, 255, 255));   // cyan
        assert_eq!(img.pixels[2], RGBA8::new(255, 0, 255, 255));  // magenta
        assert_eq!(img.pixels[3], RGBA8::new(255, 255, 0, 255));  // yellow
    }

    #[test]
    fn test_detect_color_space_non_jpeg() {
        use std::path::Path;
        assert_eq!(detect_color_space(Path::new("image.png")), ColorSpace::Rgb);
        assert_eq!(detect_color_space(Path::new("image.gif")), ColorSpace::Rgb);
    }

