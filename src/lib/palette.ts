/** File-type colours, indexed by the category the backend assigns (see layout.rs). */
export const categories = [
  { name: 'Folder', dark: '#8b7cff', light: '#6a58f5' },
  { name: 'Other', dark: '#7d8496', light: '#9aa1b2' },
  { name: 'Video', dark: '#ff6b8b', light: '#f0506f' },
  { name: 'Images', dark: '#f5b544', light: '#e0a020' },
  { name: 'Audio', dark: '#c38bff', light: '#a865f5' },
  { name: 'Archives', dark: '#ff9a5a', light: '#f27c35' },
  { name: 'Code', dark: '#3fd0c9', light: '#16b2aa' },
  { name: 'Documents', dark: '#5fa8ff', light: '#3b88f0' },
  { name: 'Programs', dark: '#9d92ff', light: '#7a6cf2' },
  { name: 'Disk images', dark: '#f37bc4', light: '#e055a8' },
  { name: 'Data & logs', dark: '#8fb3a8', light: '#6f9a8d' },
] as const;

export function categoryColor(cat: number, theme: 'dark' | 'light'): string {
  return (categories[cat] ?? categories[1])[theme];
}
