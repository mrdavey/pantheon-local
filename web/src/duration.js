export function formatDuration(ms) {
  const seconds = Math.max(0, Math.floor(ms / 1000));
  const hours = Math.floor(seconds / 3600),
    minutes = Math.floor(seconds / 60) % 60;
  return hours
    ? hours + "h " + minutes + "m"
    : minutes
      ? minutes + "m " + (seconds % 60) + "s"
      : seconds + "s";
}
