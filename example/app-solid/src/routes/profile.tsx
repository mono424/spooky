import { ProfileEdit } from '../components/ProfileEdit';
import { FeatureFlagDemo } from '../components/FeatureFlagDemo';

export default function ProfilePage() {
  return (
    <div style={{ display: 'flex', 'flex-direction': 'column', gap: '16px' }}>
      <ProfileEdit />
      <FeatureFlagDemo />
    </div>
  );
}
