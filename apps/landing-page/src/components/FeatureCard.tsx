import { Card } from "./ui/card";
import LightningIcon from "./icons/LightningIcon";
import HeartIcon from "./icons/HeartIcon";
import ShieldIcon from "./icons/ShieldIcon";

interface FeatureCardProps {
  icon: "lightning" | "heart" | "shield";
  title: string;
  description: string;
}

const iconMap = {
  lightning: LightningIcon,
  heart: HeartIcon,
  shield: ShieldIcon,
};

export default function FeatureCard({ icon, title, description }: FeatureCardProps) {
  const IconComponent = iconMap[icon];

  return (
    <Card className="p-8">
      <div className="w-12 h-12 bg-gradient-to-r from-primary-500 to-secondary-900 mb-6 flex items-center justify-center group-hover:scale-110 transition-transform duration-300">
        <IconComponent />
      </div>
      <h3 className="text-2xl font-bold mb-4 text-white">{title}</h3>
      <p className="text-gray-300 leading-relaxed">{description}</p>
    </Card>
  );
}
