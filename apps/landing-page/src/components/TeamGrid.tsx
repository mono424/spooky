import React from 'react';

interface TeamMember {
  name: string;
  role: string;
  photo: string;
  photoPosition?: string;
  xHandle?: string;
}

const teamMembers: TeamMember[] = [
  {
    name: 'Khadim Fall',
    role: 'Engineer',
    photo: '/team/khadim.jpg',
    xHandle: 'khad_im',
  },
  {
    name: 'Tim Besel',
    role: 'Engineer',
    photo: '/team/tim.jpg',
    photoPosition: 'top',
  },
];

function TeamMemberCard({ member, index }: { member: TeamMember; index: number }) {
  return (
    <div
      className="group cursor-default opacity-0 animate-fade-in"
      style={{ animationDelay: `${200 + index * 100}ms`, animationFillMode: 'forwards' }}
    >
      <div className="group-hover:-translate-y-0.5 transition-transform duration-300 ease-out">
        <div className="aspect-square overflow-hidden rounded-xl bg-white/[0.02]">
          <img
            src={member.photo}
            alt={member.name}
            className={`w-full h-full object-cover grayscale transition-[filter] duration-500 ease-out group-hover:brightness-110 ${member.photoPosition ? `object-${member.photoPosition}` : ''}`}
            loading="lazy"
          />
        </div>
        <p className="text-[15px] font-medium text-text-primary mt-4">{member.name}</p>
        <p className="text-[13px] text-text-muted mt-0.5">{member.role}</p>
        {member.xHandle && (
          <a
            href={`https://x.com/${member.xHandle}`}
            target="_blank"
            rel="noopener noreferrer"
            className="inline-flex mt-2 text-text-muted hover:text-text-primary transition-colors duration-200"
            aria-label={`${member.name} on X`}
          >
            <svg viewBox="0 0 24 24" className="w-4 h-4 fill-current">
              <path d="M18.244 2.25h3.308l-7.227 8.26 8.502 11.24H16.17l-5.214-6.817L4.99 21.75H1.68l7.73-8.835L1.254 2.25H8.08l4.713 6.231zm-1.161 17.52h1.833L7.084 4.126H5.117z" />
            </svg>
          </a>
        )}
      </div>
    </div>
  );
}

export const TeamGrid: React.FC = () => {
  return (
    <div className="max-w-4xl mx-auto px-4">
      <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 gap-x-8 gap-y-12">
        {teamMembers.map((member, i) => (
          <TeamMemberCard key={member.name} member={member} index={i} />
        ))}
      </div>
    </div>
  );
};
