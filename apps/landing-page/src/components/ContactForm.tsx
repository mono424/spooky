import { Button } from "./ui/button";
import { Input } from "./ui/input";
import { Textarea } from "./ui/textarea";
import { Card } from "./ui/card";

export default function ContactForm() {
  return (
    <Card className="p-8 max-w-md mx-auto">
      <form className="space-y-6">
        <div>
          <Input
            type="text"
            placeholder="Your Name"
          />
        </div>
        <div>
          <Input
            type="email"
            placeholder="Your Email"
          />
        </div>
        <div>
          <Textarea
            placeholder="Your Message"
            rows={4}
          />
        </div>
        <Button type="submit" className="w-full">
          Send Message
        </Button>
      </form>
    </Card>
  );
}
