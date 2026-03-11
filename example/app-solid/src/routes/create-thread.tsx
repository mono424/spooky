import { createSignal, onMount } from 'solid-js';
import { useNavigate } from '@solidjs/router';
import { CreateThreadDialog } from '../components/CreateThreadDialog';

export default function CreateThreadPage() {
  const navigate = useNavigate();
  const [showDialog, setShowDialog] = createSignal(false);

  onMount(() => {
    setShowDialog(true);
  });

  const handleClose = () => {
    setShowDialog(false);
    if (window.history.length > 1) {
      window.history.back();
    } else {
      navigate('/');
    }
  };

  return <CreateThreadDialog isOpen={showDialog()} onClose={handleClose} />;
}
