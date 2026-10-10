export function assetDisplayName(id: string): string {
  return id.replaceAll("_", " ").replace(/^./, letter => letter.toUpperCase());
}
