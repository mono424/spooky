import React, { useEffect, useState } from 'react';

// Re-exporting the Tree as the default or named to be flexible
export const MerkleTree = () => {
    const [activeNodes, setActiveNodes] = useState<number[]>([]);
    const [hashingNode, setHashingNode] = useState<number | null>(null);
    const [nodeHashes, setNodeHashes] = useState<Record<number, string>>({});
  
    // Helper to generate short random hash
    const generateHash = () => '0x' + Array.from({length: 4}, () => Math.floor(Math.random() * 16).toString(16)).join('');

    // Initial population
    useEffect(() => {
        const initial: Record<number, string> = {};
        for(let i=0; i<=6; i++) initial[i] = generateHash();
        setNodeHashes(initial);
    }, []);

    useEffect(() => {
        const sequence = async () => {
             // Reset
            setActiveNodes([]);
            setHashingNode(null);
            
            await new Promise(r => setTimeout(r, 500));

            // Step 1: Pick random leaf (3-6) and update its hash
            const leaf = Math.floor(Math.random() * 4) + 3;
            setActiveNodes([leaf]);
            setHashingNode(leaf);
            setNodeHashes(prev => ({ ...prev, [leaf]: generateHash() }));

            await new Promise(r => setTimeout(r, 800));

            // Step 2: Parent hash update
            const parent = Math.floor((leaf - 1) / 2);
            setActiveNodes((prev: number[]) => [...prev, parent]);
            setHashingNode(parent);
            setNodeHashes(prev => ({ ...prev, [parent]: generateHash() }));

            await new Promise(r => setTimeout(r, 800));

            // Step 3: Root hash update
            const root = 0;
            setActiveNodes((prev: number[]) => [...prev, root]);
            setHashingNode(root);
            setNodeHashes(prev => ({ ...prev, [root]: generateHash() }));

            await new Promise(r => setTimeout(r, 800));
            
            // Done
            setHashingNode(null);
        };

        const timer = setInterval(() => {
            sequence();
        }, 4000); 

        sequence(); 

        return () => clearInterval(timer);
    }, []);

    return (
        <div className="w-full flex flex-col items-center justify-center p-4 bg-[#0a0a0a] border border-[#333] rounded-sm relative min-h-[400px]">
            <div className="absolute top-2 right-2 flex gap-1">
                 <div className="w-1 h-1 bg-[#333] rounded-full"></div>
                 <div className="w-1 h-1 bg-[#333] rounded-full"></div>
            </div>

            <MerkleSVG 
                activeNodes={activeNodes} 
                hashingNode={hashingNode}
                nodeHashes={nodeHashes}
            />
            
            <div className="mt-4 text-[9px] text-gray-500 font-mono absolute bottom-4">
                STATUS: <span className={hashingNode !== null ? "text-purple-400 animate-pulse" : "text-gray-700"}>
                    {hashingNode !== null ? "RECOMPUTING_HASH..." : "IDLE"}
                </span>
            </div>
        </div>
    );
};

export const MerkleDashboard = () => {
  const [hashingNode, setHashingNode] = useState<number | null>(null);
  const [nodeHashes, setNodeHashes] = useState<Record<number, string>>({});
  
  const generateHash = () => '0x' + Array.from({length: 4}, () => Math.floor(Math.random() * 16).toString(16)).join('');

  useEffect(() => {
    const initial: Record<number, string> = {};
    for(let i=0; i<=6; i++) initial[i] = generateHash();
    setNodeHashes(initial);
  }, []);

  useEffect(() => {
    const sequence = async () => {
        setHashingNode(null);
        await new Promise(r => setTimeout(r, 500));
        
        setHashingNode(0);
        setNodeHashes(prev => ({ ...prev, 0: generateHash() }));

        await new Promise(r => setTimeout(r, 800));
        setHashingNode(null);
    };

    const timer = setInterval(() => {
        sequence();
    }, 3000); 

    sequence(); 

    return () => clearInterval(timer);
  }, []);

  return (
    <div className="w-full font-mono select-none h-auto">
        {/* Merged Dashboard Card - Subtler Style */}
        <div className="border-l border-white/10 bg-white/[0.02] pl-6 py-4 relative flex flex-col gap-8 min-w-0">
            
            {/* SECTION 1: ACTIVE INCANTATION */}
            <div className="flex flex-col justify-between">
                <div className="flex justify-between items-start mb-4">
                    <span className="text-[10px] font-bold text-gray-500 uppercase tracking-widest whitespace-nowrap">Active Incantation</span>
                    <div className="flex items-center gap-2">
                         <div className="w-1.5 h-1.5 rounded-full bg-green-500/50 animate-pulse"></div>
                    </div>
                </div>
                
                <div className="space-y-6">
                    <div>
                        <div className="text-[9px] font-bold text-gray-700 uppercase tracking-wider mb-1">INCANTATION_ID</div>
                        <div className="text-sm text-gray-400 font-mono tracking-wide break-all">inc_8f7a2d9c1e</div>
                    </div>
                     <div>
                        <div className="text-[9px] font-bold text-gray-700 uppercase tracking-wider mb-1">ROOT_HASH</div>
                        <div className={`text-sm font-mono tracking-wide transition-colors duration-300 break-all ${hashingNode === 0 ? "text-purple-400" : "text-gray-500"}`}>
                            {nodeHashes[0] || "0x2ab6"}
                        </div>
                    </div>
                </div>
            </div>

            {/* SECTION 2: SURQL CODE (Merged) */}
            <div className="flex-1 flex flex-col min-w-0 border-t border-white/5 pt-6">
                 
                 <pre className="text-[10px] md:text-xs text-gray-500 font-mono leading-relaxed whitespace-pre-wrap break-words">
                    <span className="text-purple-400">CREATE</span> incantation <span className="text-yellow-600">CONTENT</span> &#123;{'\n'}
                    {'  '}<span className="text-blue-400/70">query</span>: <span className="text-gray-400">"SELECT *,"</span>{'\n'}
                    {'    '}<span className="text-gray-400">"(SELECT * FROM comment WHERE Thread=$parent.id)"</span>{'\n'}
                    {'    '}<span className="text-gray-400">"FROM thread ORDER BY created_at DESC"</span>,{'\n'}
                    {'  '}<span className="text-blue-400/70">merkle_root</span>: <span className="text-orange-300/70">{nodeHashes[0] || "0x3e4d..."}</span>...{'\n'}
                    &#125;;
                 </pre>
                 
                 <div className="mt-4 pt-4 text-[10px] text-gray-600 leading-relaxed">
                    <span className="text-gray-400 font-bold">Note:</span> Live queries subscribe to root hashes. Data syncs when client/server hashes match.
                 </div>
            </div>
        </div>
    </div>
  );
};

// Keeping the original name for backward compat if needed, but we should switch to MerkleTree
export const MerkleTreeAnimation = MerkleTree;

// SVG Sub-component for clean separation
const MerkleSVG = ({ 
    activeNodes, 
    hashingNode,
    nodeHashes 
}: { 
    activeNodes: number[], 
    hashingNode: number | null,
    nodeHashes: Record<number, string>
}) => {
    // Layout Constants
    const WIDTH = 400;
    const HEIGHT = 240;
    const LEVEL_Y = [30, 110, 190]; // Y coords for Root, L1, L2
    
    // Static structure, label becomes subtitle, value is hash
    const nodes = [
        { id: 0, x: WIDTH / 2, y: LEVEL_Y[0], title: "ROOT" },
        { id: 1, x: WIDTH * 0.25, y: LEVEL_Y[1], title: "THREAD_A" },
        { id: 2, x: WIDTH * 0.75, y: LEVEL_Y[1], title: "THREAD_B" },
        { id: 3, x: WIDTH * 0.125, y: LEVEL_Y[2], title: "CMT_3" },
        { id: 4, x: WIDTH * 0.375, y: LEVEL_Y[2], title: "CMT_4" },
        { id: 5, x: WIDTH * 0.625, y: LEVEL_Y[2], title: "CMT_5" },
        { id: 6, x: WIDTH * 0.875, y: LEVEL_Y[2], title: "CMT_6" },
    ];

    // Connections [parent, child]
    const links = [
        [0, 1], [0, 2],
        [1, 3], [1, 4],
        [2, 5], [2, 6]
    ];

    const getBoxStyle = (id: number) => {
        const isHashing = hashingNode === id;
        const isActive = activeNodes.includes(id);
        
        return {
            fill: isHashing ? "rgba(88, 28, 135, 0.4)" : "#0f0f0f",
            stroke: isHashing ? "#a855f7" : (isActive ? "#581c87" : "#333"),
            strokeWidth: isHashing ? 2 : 1,
            filter: isHashing ? "url(#glow)" : "none"
        };
    };

    return (
        <svg width="100%" height="100%" viewBox={`0 0 ${WIDTH} ${HEIGHT}`} className="max-w-md w-full h-full">
            <defs>
                <filter id="glow" x="-20%" y="-20%" width="140%" height="140%">
                    <feGaussianBlur stdDeviation="3" result="blur" />
                    <feComposite in="SourceGraphic" in2="blur" operator="over" />
                </filter>
            </defs>

            {/* Connecting Lines (Elbow style) */}
            {links.map(([parentId, childId]) => {
                const parent = nodes[parentId];
                const child = nodes[childId];
                const isActive = activeNodes.includes(childId); // Highlight if child is part of flow
                
                // Elbow path logic
                const midY = (parent.y + child.y) / 2;
                const pathD = `M ${parent.x} ${parent.y + 20} V ${midY} H ${child.x} V ${child.y - 20}`;

                return (
                    <path 
                        key={`${parentId}-${childId}`}
                        d={pathD}
                        fill="none"
                        stroke={isActive ? "#a855f7" : "#333"}
                        strokeWidth={isActive ? 1.5 : 1}
                        className="transition-colors duration-300"
                    />
                );
            })}

            {/* Nodes */}
            {nodes.map(node => (
                <g key={node.id} className="transition-all duration-300">
                    {/* Box */}
                    <rect 
                        x={node.x - 40} 
                        y={node.y - 20} 
                        width={80} 
                        height={40} 
                        rx={4}
                        className="transition-all duration-300"
                        {...getBoxStyle(node.id)}
                    />
                    
                    {/* Text Labels */}
                    <text 
                        x={node.x} 
                        y={node.y - 5} 
                        textAnchor="middle" 
                        fill="#666" 
                        fontSize="6" 
                        fontFamily="monospace"
                        className="uppercase"
                    >
                        {node.title}
                    </text>
                    <text 
                        x={node.x} 
                        y={node.y + 8} 
                        textAnchor="middle" 
                        fill={hashingNode === node.id ? "#fff" : "#ccc"} 
                        fontSize="8" 
                        fontWeight="bold"
                        fontFamily="monospace"
                    >
                        {nodeHashes[node.id] || "..."}
                    </text>
                </g>
            ))}
        </svg>
    );
};
