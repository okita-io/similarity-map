/** RF chapter multi-pass presets — window/stride pairs match `generate:similarity_map:passes`. */
export const RF_CHAPTER_PRESETS = {
  act_fine: {
    label: "Romance Factory — act fine",
    displayPass: { windowSize: 50, stride: 10 },
    passes: [
      { name: "act_50_10", scope: "act", windowSize: 50, stride: 10 },
      { name: "act_100_25", scope: "act", windowSize: 100, stride: 25 },
    ],
  },
  chapter_coarse: {
    label: "RF — chapter coarse",
    displayPass: { windowSize: 200, stride: 50 },
    passes: [
      { name: "chapter_200_50", scope: "chapter", windowSize: 200, stride: 50 },
      { name: "chapter_400_100", scope: "chapter", windowSize: 400, stride: 100 },
    ],
  },
  full_multi_pass: {
    label: "RF — full multi-pass",
    displayPass: { windowSize: 50, stride: 10 },
    passes: [
      { name: "act_50_10", scope: "act", windowSize: 50, stride: 10 },
      { name: "act_100_25", scope: "act", windowSize: 100, stride: 25 },
      { name: "chapter_200_50", scope: "chapter", windowSize: 200, stride: 50 },
      { name: "chapter_400_100", scope: "chapter", windowSize: 400, stride: 100 },
    ],
  },
};

export const DEFAULT_RF_CHAPTER_PRESET = "full_multi_pass";
