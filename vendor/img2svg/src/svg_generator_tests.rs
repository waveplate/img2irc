#[cfg(test)]
use super::*;
#[cfg(test)]
use crate::vectorizer::{Point, Curve, VectorizedData};
#[cfg(test)]
use std::fs;
#[cfg(test)]
use std::path::PathBuf;

    fn create_test_vectorized_data() -> VectorizedData {
        VectorizedData {
            width: 100,
            height: 100,
            background_color: (255, 255, 255, 255),
            curves: vec![
                Curve {
                    points: vec![
                        Point { x: 10.0, y: 10.0 },
                        Point { x: 90.0, y: 10.0 },
                        Point { x: 90.0, y: 90.0 },
                        Point { x: 10.0, y: 90.0 },
                    ],
                    color: (255, 0, 0, 255),
                    is_closed: true,
                    subpaths: vec![],
                },
                Curve {
                    points: vec![
                        Point { x: 30.0, y: 30.0 },
                        Point { x: 70.0, y: 30.0 },
                        Point { x: 70.0, y: 70.0 },
                        Point { x: 30.0, y: 70.0 },
                    ],
                    color: (0, 0, 255, 255),
                    is_closed: true,
                    subpaths: vec![],
                },
            ],
        }
    }

    fn create_test_vectorized_data_with_subpaths() -> VectorizedData {
        VectorizedData {
            width: 100,
            height: 100,
            background_color: (255, 255, 255, 255),
            curves: vec![
                Curve {
                    points: vec![],
                    color: (255, 0, 0, 255),
                    is_closed: true,
                    subpaths: vec![
                        vec![
                            Point { x: 10.0, y: 10.0 },
                            Point { x: 40.0, y: 10.0 },
                            Point { x: 40.0, y: 40.0 },
                            Point { x: 10.0, y: 40.0 },
                        ],
                        vec![
                            Point { x: 60.0, y: 60.0 },
                            Point { x: 90.0, y: 60.0 },
                            Point { x: 90.0, y: 90.0 },
                            Point { x: 60.0, y: 90.0 },
                        ],
                    ],
                },
            ],
        }
    }

    #[test]
    fn test_generate_svg_basic() {
        let data = create_test_vectorized_data();
        let output_path = PathBuf::from("/tmp/test_output.svg");

        let result = generate_svg(&data, &output_path);
        assert!(result.is_ok());

        assert!(output_path.exists());

        let content = fs::read_to_string(&output_path).unwrap();
        assert!(content.contains("<svg"));
        assert!(content.contains("width=\"100\""));
        assert!(content.contains("height=\"100\""));
        assert!(content.contains("</svg>"));

        let _ = fs::remove_file(&output_path);
    }

    #[test]
    fn test_generate_svg_with_background_color() {
        let data = create_test_vectorized_data();
        let output_path = PathBuf::from("/tmp/test_background.svg");

        let result = generate_svg(&data, &output_path);
        assert!(result.is_ok());

        let content = fs::read_to_string(&output_path).unwrap();
        assert!(content.contains("#ffffff"));

        let _ = fs::remove_file(&output_path);
    }

    #[test]
    fn test_generate_svg_with_curves() {
        let data = create_test_vectorized_data();
        let output_path = PathBuf::from("/tmp/test_curves.svg");

        let result = generate_svg(&data, &output_path);
        assert!(result.is_ok());

        let content = fs::read_to_string(&output_path).unwrap();
        assert!(content.contains("#ff0000"));
        assert!(content.contains("#0000ff"));
        assert!(content.contains("<path"));

        let _ = fs::remove_file(&output_path);
    }

    #[test]
    fn test_generate_svg_advanced() {
        let data = create_test_vectorized_data();
        let output_path = PathBuf::from("/tmp/test_advanced.svg");

        let result = generate_svg_advanced(&data, &output_path);
        assert!(result.is_ok());
        assert!(output_path.exists());

        let _ = fs::remove_file(&output_path);
    }

    #[test]
    fn test_generate_svg_with_subpaths() {
        let data = create_test_vectorized_data_with_subpaths();
        let output_path = PathBuf::from("/tmp/test_subpaths.svg");

        let result = generate_svg(&data, &output_path);
        assert!(result.is_ok());

        let content = fs::read_to_string(&output_path).unwrap();
        assert!(content.contains("<path"));

        let _ = fs::remove_file(&output_path);
    }

    #[test]
    fn test_generate_svg_empty_curves() {
        let data = VectorizedData {
            width: 50,
            height: 50,
            background_color: (128, 128, 128, 255),
            curves: vec![],
        };
        let output_path = PathBuf::from("/tmp/test_empty.svg");

        let result = generate_svg(&data, &output_path);
        assert!(result.is_ok());

        let content = fs::read_to_string(&output_path).unwrap();
        assert!(content.contains("<svg"));
        assert!(content.contains("#808080"));

        let _ = fs::remove_file(&output_path);
    }

    #[test]
    fn test_generate_svg_with_alpha() {
        let data = VectorizedData {
            width: 50,
            height: 50,
            background_color: (255, 255, 255, 128),
            curves: vec![
                Curve {
                    points: vec![
                        Point { x: 10.0, y: 10.0 },
                        Point { x: 40.0, y: 10.0 },
                        Point { x: 40.0, y: 40.0 },
                        Point { x: 10.0, y: 40.0 },
                    ],
                    color: (255, 0, 0, 64),
                    is_closed: true,
                    subpaths: vec![],
                },
            ],
        };
        let output_path = PathBuf::from("/tmp/test_alpha.svg");
        let result = generate_svg(&data, &output_path);
        assert!(result.is_ok());

        let content = fs::read_to_string(&output_path).unwrap();
        assert!(content.contains("fill-opacity=\"0.5\""));

        let _ = fs::remove_file(&output_path);
    }

    #[test]
    fn test_generate_svg_hierarchical_evenodd() {
        let data = VectorizedData {
            width: 100,
            height: 100,
            background_color: (255, 255, 255, 255),
            curves: vec![
                Curve {
                    points: vec![],
                    color: (255, 0, 0, 255),
                    is_closed: true,
                    subpaths: vec![
                        vec![
                            Point { x: 10.0, y: 10.0 },
                            Point { x: 90.0, y: 10.0 },
                            Point { x: 90.0, y: 90.0 },
                            Point { x: 10.0, y: 90.0 },
                        ],
                        vec![
                            Point { x: 30.0, y: 30.0 },
                            Point { x: 30.0, y: 70.0 },
                            Point { x: 70.0, y: 70.0 },
                            Point { x: 70.0, y: 30.0 },
                        ],
                    ],
                },
            ],
        };
        let output_path = PathBuf::from("/tmp/test_evenodd.svg");
        let result = generate_svg(&data, &output_path);
        assert!(result.is_ok());

        let content = fs::read_to_string(&output_path).unwrap();
        assert!(content.contains("fill-rule=\"evenodd\""));

        let _ = fs::remove_file(&output_path);
    }

    #[test]
    fn test_generate_svg_to_matches_file_output() {
        let data = VectorizedData {
            width: 100,
            height: 100,
            background_color: (255, 255, 255, 255),
            curves: vec![
                Curve {
                    points: vec![],
                    color: (255, 0, 0, 255),
                    is_closed: true,
                    subpaths: vec![
                        vec![
                            Point { x: 10.0, y: 10.0 },
                            Point { x: 90.0, y: 10.0 },
                            Point { x: 90.0, y: 90.0 },
                            Point { x: 10.0, y: 90.0 },
                        ],
                    ],
                },
            ],
        };
        let file_path = PathBuf::from("/tmp/test_svg_to.svg");
        generate_svg(&data, &file_path).unwrap();
        let from_file = std::fs::read_to_string(&file_path).unwrap();

        let mut buffer = Vec::new();
        generate_svg_to(&data, &mut buffer).unwrap();
        let from_writer = String::from_utf8(buffer).unwrap();

        assert_eq!(from_file, from_writer);
        let _ = std::fs::remove_file(&file_path);
    }

    #[test]
    fn test_write_subpath_to_emits_expected_svg() {
        let pts = vec![
            Point { x: 0.0, y: 0.0 },
            Point { x: 5.0, y: 0.0 },
            Point { x: 5.0, y: 5.0 },
        ];

        let mut buf = Vec::new();
        write_subpath_to(&mut buf, &pts, true).unwrap();
        let svg = String::from_utf8(buf).unwrap();

        assert_eq!(svg, "M0 0L5 0L5 5Z");
    }

    #[test]
    fn test_write_multi_path_to_skips_short_subpaths() {
        let subpaths = vec![
            vec![
                Point { x: 0.0, y: 0.0 },
                Point { x: 10.0, y: 0.0 },
            ], // Only 2 points — should be skipped
            vec![
                Point { x: 20.0, y: 20.0 },
                Point { x: 30.0, y: 20.0 },
                Point { x: 30.0, y: 30.0 },
            ], // 3 points — included
        ];

        let mut buf = Vec::new();
        write_multi_path_to(&mut buf, &subpaths).unwrap();
        let svg = String::from_utf8(buf).unwrap();

        assert!(!svg.contains("M0 0"));
        assert!(svg.contains("M20 20"));
    }

