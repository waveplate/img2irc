import sys

def main():
    with open('src/gpu.rs', 'r') as f:
        content = f.read()

    start_str = "    // Iterate manually to construct batches on the fly"
    end_str = "    if batch_count > 0 {"

    start_idx = content.find(start_str)
    end_idx = content.find(end_str)

    if start_idx == -1 or end_idx == -1:
        print("Could not find start or end string.")
        return

    replacement = """    // Flatten blocks to iterate over them
    let mut flat_blocks = Vec::with_capacity(total_blocks);
    for row_idx in 0..image.block.len() {
        for col_idx in 0..if image.block.is_empty() {
            0
        } else {
            image.block[0].len()
        } {
            flat_blocks.push(&image.block[row_idx][col_idx]);
        }
    }

    use rayon::prelude::*;

    for chunk in flat_blocks.chunks(max_blocks_per_batch) {
        let chunk_size = chunk.len();
        
        let batch_blocks: Vec<GpuBlockPixel> = chunk
            .par_iter()
            .flat_map(|blk| {
                let mut pixels = Vec::with_capacity((gh * gw) as usize);
                for y in 0..gh {
                    for x in 0..gw {
                        let px_orig = blk.pixels[y][x].orig;
                        let rgb = crate::draw::unpack_rgb(px_orig);
                        let p_r = rgb[0] as u32;
                        let p_g = rgb[1] as u32;
                        let p_b = rgb[2] as u32;
                        let p_sq = p_r * p_r + p_g * p_g + p_b * p_b;
                        pixels.push(GpuBlockPixel {
                            r: p_r,
                            g: p_g,
                            b: p_b,
                            sq: p_sq,
                        });
                    }
                }
                pixels
            })
            .collect();

        total_prep_time += t_prep.elapsed();
        let t_batch_gpu = std::time::Instant::now();
        
        process_batch(
            device,
            queue,
            &compute_pipeline,
            &bind_group,
            &blocks_buf,
            &results_buf,
            &staging_buf,
            &batch_blocks,
            chunk_size,
            &mut final_results,
        );
        
        total_gpu_time += t_batch_gpu.elapsed();
        t_prep = std::time::Instant::now();
    }

"""

    new_content = content[:start_idx] + replacement + content[end_idx:]
    with open('src/gpu.rs', 'w') as f:
        f.write(new_content)

if __name__ == "__main__":
    main()
