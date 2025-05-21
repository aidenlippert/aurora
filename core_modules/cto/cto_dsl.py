# ~/aurora_project/core_modules/cto/cto_dsl.py
# Corrected imports for Pylance strict mode
from typing import TypeVar, Any, Callable, Type, Set # Import Set

# Import the foundational types from cto_types.py
from .cto_types import Object, Morphism, BasicObject, BasicMorphism, Category


# Define a type for a "Computational Service" as a specialized Object
SvcInput = TypeVar('SvcInput') # Type for the input data of the service
SvcOutput = TypeVar('SvcOutput') # Type for the output data of the service

# Removed explicit generics from ComputationalService class definition,
# as it inherits Generic from BasicObject.
class ComputationalService(BasicObject[Callable[[SvcInput], SvcOutput]]):
    """
    Represents a computational microservice or function as an Object in AURORA.
    The 'value_type' of this object is `Callable[[SvcInput], SvcOutput]`,
    and its encapsulated `value` is the actual executable Python callable.
    """
    def __init__(self, name: str, operation: Callable[[SvcInput], SvcOutput],
                 input_type_obj: Object[SvcInput], output_type_obj: Object[SvcOutput]):
        # The 'value_type' of this BasicObject is Callable, but its 'value' is the specific operation
        super().__init__(type(operation)) # The object represents the callable type
        self.name = name
        self._operation = operation # Store the actual callable
        self.input_type_obj = input_type_obj # The Object representing the input type
        self.output_type_obj = output_type_obj # The Object representing the output type

    # Public property to access the encapsulated operation.
    # This resolves Pylance's `reportPrivateUsage` warning for `_operation`.
    @property
    def operation(self) -> Callable[[SvcInput], SvcOutput]:
        return self._operation

    # Override the __call__ method to execute the service operation directly
    def __call__(self, input_data: SvcInput) -> SvcOutput:
        """Execute the encapsulated service operation."""
        return self._operation(input_data)

    def __str__(self) -> str:
        return f"Service({self.name}: {self.input_type_obj.value_type.__name__} -> {self.output_type_obj.value_type.__name__})"

class ServiceInvocation(BasicMorphism[SvcInput, SvcOutput]):
    """
    Represents the invocation of a ComputationalService as a Morphism.
    This morphism transforms input data `SvcInput` to output data `SvcOutput`.
    """
    def __init__(self, service_instance: ComputationalService[SvcInput, SvcOutput]):
        super().__init__(
            domain_object=service_instance.input_type_obj,  # The Object representing the input type
            codomain_object=service_instance.output_type_obj, # The Object representing the output type
            operation=service_instance.operation # Access via public property now
        )
        self.service_instance = service_instance

    def __str__(self) -> str:
        return f"Invoke({self.service_instance.name})"


# This is just a start. Future DSL constructs will use lark to parse
# custom syntax for defining services, contracts, and compositions.
# Example:
if __name__ == "__main__":
    # Define base type objects
    int_type_obj = BasicObject(int)
    str_type_obj = BasicObject(str)
    # Adding a float type object as well
    float_type_obj = BasicObject(float)

    # Define a simple service operation
    def increment_and_to_str(num: int) -> str:
        return str(num + 1)

    # Create a ComputationalService object
    my_first_service = ComputationalService(
        name="IncrementAndToString",
        operation=increment_and_to_str,
        input_type_obj=int_type_obj,
        output_type_obj=str_type_obj
    )

    # Invoke the service directly (as it's a callable object)
    result_from_service = my_first_service(5)
    print(f"Service '{my_first_service.name}' invoked with 5: '{result_from_service}' (type: {type(result_from_service).__name__})")

    # Create a ServiceInvocation morphism from the service
    invoke_service_morphism = ServiceInvocation(my_first_service)

    # Apply the morphism
    result_from_morphism = invoke_service_morphism(10)
    print(f"Morphism '{invoke_service_morphism.domain.value_type.__name__}' -> '{invoke_service_morphism.codomain.value_type.__name__}' invoked with 10: '{result_from_morphism}' (type: {type(result_from_morphism).__name__})")

    # Example of a concrete Category to test composition
    # The generic types for Category are ObjValue (Type[Any]), MorphismInput (Any), MorphismOutput (Any)
    class SimpleServiceCategory(Category[Type[Any], Any, Any]):
        def objects(self) -> Set[Object[Any]]: # Corrected type hint
            return {int_type_obj, str_type_obj, float_type_obj} # Use existing type objects

        # Parameter names `domain` and `codomain` now match the abstract method in cto_types.py
        def morphisms(self, domain: Object[Any], codomain: Object[Any]) -> Set[Morphism[Any, Any]]: # Corrected type hint
            found_morphisms: Set[Morphism[Any, Any]] = set()
            if domain == int_type_obj and codomain == str_type_obj:
                found_morphisms.add(ServiceInvocation(my_first_service)) # Add the service invocation
            
            # Add other example morphisms as needed for composition tests
            def str_to_len(s: str) -> int:
                return len(s)
            len_service = ComputationalService(
                name="StringLength",
                operation=str_to_len,
                input_type_obj=str_type_obj,
                output_type_obj=int_type_obj
            )
            found_morphisms.add(ServiceInvocation(len_service))

            def int_to_float(i: int) -> float:
                return float(i)
            float_service = ComputationalService(
                name="IntToFloat",
                operation=int_to_float,
                input_type_obj=int_type_obj,
                output_type_obj=float_type_obj
            )
            found_morphisms.add(ServiceInvocation(float_service))

            return found_morphisms

    service_cat = SimpleServiceCategory()

    # Now, compose with another simple morphism (e.g., str_to_len from cto_types.py)
    # Composition: int -> str -> int
    # We need to retrieve the specific morphisms from the category
    int_to_str_m = next(iter(service_cat.morphisms(int_type_obj, str_type_obj))) # Get the int->str morphism
    str_to_len_m = next(iter(service_cat.morphisms(str_type_obj, int_type_obj))) # Get the str->int morphism

    int_to_len_morphism = service_cat.compose(int_to_str_m, str_to_len_m)
    final_composed_result = int_to_len_morphism(4567)
    print(f"Composed service (int -> str -> int) applied to 4567: {final_composed_result} (type: {type(final_composed_result).__name__})")

    # Another composition example: int -> str -> str
    def str_to_upper(s: str) -> str:
        return s.upper()

    upper_service = ComputationalService(
        name="StringToUpper",
        operation=str_to_upper,
        input_type_obj=str_type_obj,
        output_type_obj=str_type_obj
    )
    invoke_upper_morphism = ServiceInvocation(upper_service)
    
    # Add the upper_service to the category for proper retrieval, or create it directly if only for example
    # For a proper category, you'd add this to the set of available morphisms in SimpleServiceCategory
    # For this example, we'll just use the instance directly
    int_to_upper_str_morphism = service_cat.compose(int_to_str_m, invoke_upper_morphism)
    final_composed_result_upper = int_to_upper_str_morphism(20)
    print(f"Composed service (int -> str -> str) applied to 20: '{final_composed_result_upper}' (type: {type(final_composed_result_upper).__name__})")