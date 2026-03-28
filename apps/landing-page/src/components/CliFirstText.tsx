import { ScrollRevealText } from './ScrollRevealText';

export function CliFirstText() {
  return (
    <ScrollRevealText
      className="text-2xl md:text-3xl font-semibold leading-snug"
      segments={[
        { text: 'Dev to prod is one command. ', preRevealed: true },
        { text: 'No dashboards. No pipelines. No config files. Just spky deploy. Your entire stack goes live in seconds.' },
      ]}
    />
  );
}
