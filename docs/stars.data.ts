export default {
  async load() {
    const res = await fetch("https://api.github.com/repos/raine/workmux");
    if (!res.ok) return null;
    const data = await res.json();
    return data.stargazers_count as number;
  },
};
