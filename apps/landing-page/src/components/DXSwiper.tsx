import React, { useState, useEffect, useCallback } from 'react';
import { SchemaWorkflowEyecatcher } from './SchemaWorkflowEyecatcher';
import { DXPane1 } from './DXPane1';
import { DXPane2 } from './DXPane2';

export const DXSwiper: React.FC = () => {
  const [currentPane, setCurrentPane] = useState(0);
  const [isTransitioning, setIsTransitioning] = useState(false);
  const [startX, setStartX] = useState(0);
  const [isDragging, setIsDragging] = useState(false);

  const totalPanes = 3;

  // Navigation handlers
  const handleNext = useCallback(() => {
    if (isTransitioning || currentPane >= totalPanes - 1) return;
    setIsTransitioning(true);
    const nextPane = Math.min(currentPane + 1, totalPanes - 1);
    setCurrentPane(nextPane);
    setTimeout(() => setIsTransitioning(false), 400);
  }, [currentPane, totalPanes, isTransitioning]);

  const handlePrev = useCallback(() => {
    if (isTransitioning || currentPane <= 0) return;
    setIsTransitioning(true);
    setCurrentPane((prev) => Math.max(prev - 1, 0));
    setTimeout(() => setIsTransitioning(false), 400);
  }, [currentPane, isTransitioning]);

  const handleDotClick = useCallback(
    (index: number) => {
      if (isTransitioning || index === currentPane) return;
      setIsTransitioning(true);
      setCurrentPane(index);
      setTimeout(() => setIsTransitioning(false), 400);
    },
    [currentPane, isTransitioning]
  );

  // Keyboard navigation
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (isTransitioning) return;

      switch (e.key) {
        case 'ArrowLeft':
          e.preventDefault();
          handlePrev();
          break;
        case 'ArrowRight':
          e.preventDefault();
          handleNext();
          break;
        case 'Home':
          e.preventDefault();
          if (currentPane !== 0) {
            setIsTransitioning(true);
            setCurrentPane(0);
            setTimeout(() => setIsTransitioning(false), 400);
          }
          break;
        case 'End':
          e.preventDefault();
          if (currentPane !== totalPanes - 1) {
            setIsTransitioning(true);
            setCurrentPane(totalPanes - 1);
            setTimeout(() => setIsTransitioning(false), 400);
          }
          break;
      }
    };

    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, [currentPane, totalPanes, isTransitioning, handleNext, handlePrev]);

  // Touch gesture handlers
  const handlePointerDown = (e: React.PointerEvent) => {
    setStartX(e.clientX);
    setIsDragging(true);
  };

  const handlePointerMove = (e: React.PointerEvent) => {
    if (!isDragging || isTransitioning) return;
    const diff = e.clientX - startX;

    // 50px threshold to trigger navigation
    if (Math.abs(diff) > 50) {
      if (diff > 0 && currentPane > 0) {
        handlePrev();
      } else if (diff < 0 && currentPane < totalPanes - 1) {
        handleNext();
      }
      setIsDragging(false);
    }
  };

  const handlePointerUp = () => {
    setIsDragging(false);
  };

  return (
    <div
      role="region"
      aria-label="Developer Experience Features"
      className="mt-12"
    >
      {/* Swiper container - MUST hide overflow and set relative positioning */}
      <div
        className="swiper-container"
        style={{
          position: 'relative',
          overflow: 'hidden',
          width: '100%',
        }}
        onPointerDown={handlePointerDown}
        onPointerMove={handlePointerMove}
        onPointerUp={handlePointerUp}
        onPointerCancel={handlePointerUp}
      >
        {/* Track with horizontal slide animation */}
        <div
          className="swiper-track"
          style={{
            display: 'flex',
            transform: `translateX(-${currentPane * 100}%)`,
            transition: 'transform 400ms ease-in-out',
            willChange: 'transform',
          }}
        >
          {/* Pane 0: Code Editor & Features */}
          <div
            className="swiper-slide"
            style={{
              minWidth: '100%',
              width: '100%',
              flexShrink: 0,
            }}
            aria-hidden={currentPane !== 0}
          >
            <DXPane1 />
          </div>

          {/* Pane 1: Schema-First Development */}
          <div
            className="swiper-slide"
            style={{
              minWidth: '100%',
              width: '100%',
              flexShrink: 0,
            }}
            aria-hidden={currentPane !== 1}
          >
            <SchemaWorkflowEyecatcher />
          </div>

          {/* Pane 2: DevTools Simulation */}
          <div
            className="swiper-slide"
            style={{
              minWidth: '100%',
              width: '100%',
              flexShrink: 0,
            }}
            aria-hidden={currentPane !== 2}
          >
            <DXPane2 />
          </div>
        </div>
      </div>

      {/* Accessibility: Announce pane changes */}
      <div
        className="sr-only"
        role="status"
        aria-live="polite"
        aria-atomic="true"
      >
        Showing feature {currentPane + 1} of {totalPanes}
      </div>

      {/* Navigation Controls */}
      <div className="flex items-center justify-center gap-6 mt-12">
        {/* Previous button */}
        <button
          onClick={handlePrev}
          disabled={currentPane === 0}
          aria-label="Previous feature"
          className="swiper-arrow"
        >
          <svg
            xmlns="http://www.w3.org/2000/svg"
            width="20"
            height="20"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            strokeWidth="2"
            strokeLinecap="round"
            strokeLinejoin="round"
          >
            <path d="m15 18-6-6 6-6" />
          </svg>
        </button>

        {/* Dot indicators */}
        <div className="swiper-dots" role="tablist" aria-label="Feature navigation">
          {Array.from({ length: totalPanes }).map((_, index) => (
            <button
              key={index}
              onClick={() => handleDotClick(index)}
              aria-label={`Go to feature ${index + 1}`}
              aria-current={currentPane === index ? 'true' : 'false'}
              role="tab"
              aria-selected={currentPane === index}
              className="swiper-dot"
            />
          ))}
        </div>

        {/* Next button */}
        <button
          onClick={handleNext}
          disabled={currentPane === totalPanes - 1}
          aria-label="Next feature"
          className="swiper-arrow"
        >
          <svg
            xmlns="http://www.w3.org/2000/svg"
            width="20"
            height="20"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            strokeWidth="2"
            strokeLinecap="round"
            strokeLinejoin="round"
          >
            <path d="m9 18 6-6-6-6" />
          </svg>
        </button>
      </div>
    </div>
  );
};
