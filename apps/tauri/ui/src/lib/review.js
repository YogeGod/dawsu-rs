// The Write Review's .wav and volume sections, from the review (or Export preview) Rust sends:
// mixes is every staged .wav; createdMixes and reusedMixes split it; volumeSummary says whether
// volumes go on each object in the .osu or on timing lines. Rust also adds the volume lines to
// details, so they are taken out there and shown once, under Volumes.
export function reviewMixSections(review) {
  const mixes = review?.mixes ?? [];
  const volumes = review?.volumeSummary ?? [];
  const shown = new Set(volumes);
  const details = (review?.details ?? []).filter(line => !shown.has(line));
  const rows = paths => (paths ?? []).map(path => ({ path, index: mixes.indexOf(path) }));
  const split = Array.isArray(review?.createdMixes) || Array.isArray(review?.reusedMixes);
  const sections = (split ? [
    { title: "New .wav files dawsu! creates", rows: rows(review.createdMixes) },
    { title: "Existing .wav files reused (left as they are)", rows: rows(review.reusedMixes) },
  ] : [{ title: "Sample mixes", rows: rows(mixes) }]).filter(section => section.rows.length);
  return { sections, volumes, details };
}
