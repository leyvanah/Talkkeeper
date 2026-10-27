import React from 'react';
import { ChevronLeft } from 'lucide-react';
import { cn } from '@/lib/utils';
import { ProgressIndicator } from './shared/ProgressIndicator';
import { ONBOARDING_STEPS, useOnboarding } from '@/contexts/OnboardingContext';
import type { OnboardingContainerProps } from '@/types/onboarding';

/**
 * The frame of every onboarding screen: the step indicator, a "back" arrow,
 * the title and the content.
 *
 * Forward is each screen's own button, and only that: an arrow beside it was
 * a second way on, one step had both.
 */
export function OnboardingContainer({
  title,
  description,
  children,
  step,
  stepOffset = 0,
  hideProgress = false,
  className,
  showNavigation = false,
  onPrevious,
  canGoPrevious = true,
}: OnboardingContainerProps) {
  const { goToStep, goPrevious } = useOnboarding();

  const handlePrevious = () => {
    if (onPrevious) {
      onPrevious();
    } else {
      goPrevious();
    }
  };

  const handleStepClick = (s: number) => {
    goToStep(s + stepOffset);
  };

  return (
    <div className="fixed inset-0 bg-gray-50 flex items-center justify-center z-50 overflow-hidden">
      <div className={cn(
        'w-full max-w-2xl h-full max-h-screen flex flex-col px-6 py-6',
        className,
      )}>
        {/* Progress Indicator with Navigation - Fixed */}
        {step && !hideProgress && (
          <div className="mb-2 relative flex-shrink-0">
            {/* Back only; forward is the screen's own button */}
            {showNavigation && (
              <div className="absolute top-1/2 -translate-y-1/2 left-0 right-0 flex justify-start pointer-events-none">
                <button
                  onClick={handlePrevious}
                  disabled={!canGoPrevious || step === 1}
                  className={cn(
                    'pointer-events-auto w-8 h-8 rounded-full bg-white border border-gray-200 shadow-sm flex items-center justify-center transition-all duration-200',
                    canGoPrevious && step !== 1
                      ? 'hover:bg-gray-50 hover:shadow-md hover:scale-110 text-gray-700'
                      : 'opacity-0 cursor-not-allowed'
                  )}
                >
                  <ChevronLeft className="w-4 h-4" />
                </button>
              </div>
            )}

            {/* Progress Indicator: the same number of steps on every screen */}
            <ProgressIndicator current={step} total={ONBOARDING_STEPS} onStepClick={handleStepClick} />
          </div>
        )}

        {/* Header - Fixed */}
        <div className="mb-4 text-center space-y-3 flex-shrink-0">
          <h1 className="text-4xl font-semibold text-gray-900 animate-fade-in-up">{title}</h1>
          {description && (
            <p className="text-base text-gray-600 max-w-md mx-auto animate-fade-in-up delay-75">
              {description}
            </p>
          )}
        </div>

        {/* Content - Scrollable */}
        <div className="flex-1 overflow-y-auto pr-2">
          <div className="space-y-6">{children}</div>
        </div>
      </div>
    </div>
  );
}
