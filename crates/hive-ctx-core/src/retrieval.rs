use rayon::prelude::*;

pub fn top_k(scores: &[f32], k: usize) -> Vec<usize> {
  let mut idx: Vec<usize> = (0..scores.len()).collect();
  idx.par_sort_unstable_by(|a, b| scores[*b].total_cmp(&scores[*a]));
  idx.truncate(k.min(idx.len()));
  idx
}

